//! Type/path/call substitution tables applied to the derived sync expansion.
//!
//! The source is written in async form; entries map the async spelling to the
//! sync spelling. Matching is purely syntactic (token-level, after parse
//! normalization). There is no name resolution: aliased spellings must be
//! added to the table by the user.

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::{Ident, Path, PathArguments, PathSegment, Token, Type};

use crate::config::Config;

#[derive(Default)]
pub struct SubstTable {
    /// Whole-type substitutions, keyed by the normalized token string of the
    /// async (source) spelling.
    types: Vec<(String, Type)>,
    /// Plain-path substitutions (types whose async spelling is a bare path
    /// with no generic arguments). Used for expression paths
    /// (`AsyncStore::new()`), struct literals, patterns, trait bounds and
    /// trait-impl targets, where the path may extend past the matched prefix.
    paths: Vec<(Path, Path)>,
    /// Single-ident → single-ident subset of `paths`, safe to apply inside
    /// macro token streams.
    idents: Vec<(String, Ident)>,
    /// Call renames: async (source) name → sync name.
    calls: Vec<(String, Ident)>,
    /// Textual spellings of `paths` and `calls` entries, applied to doc
    /// comments at identifier boundaries (`[`AsyncStore`]` → `[`Store`]`).
    doc_texts: Vec<(String, String)>,
}

impl SubstTable {
    pub fn build(cfg: &Config) -> Self {
        let mut table = SubstTable::default();
        for entry in &cfg.sync.types {
            table.register_type(&entry.async_, &entry.sync, false);
        }
        for swap in &cfg.sync.calls {
            table
                .calls
                .push((swap.async_.to_string(), swap.sync.clone()));
            table
                .doc_texts
                .push((swap.async_.to_string(), swap.sync.to_string()));
        }
        table
    }

    fn register_type(&mut self, from: &Type, to: &Type, first: bool) {
        let type_entry = (from.to_token_stream().to_string(), to.clone());
        if first {
            self.types.insert(0, type_entry);
        } else {
            self.types.push(type_entry);
        }

        let (Type::Path(from), Type::Path(to)) = (from, to) else {
            return;
        };
        if from.qself.is_some() || to.qself.is_some() {
            return;
        }

        let path_entry = (from.path.clone(), to.path.clone());
        let doc_entry = (path_doc_text(&from.path), path_doc_text(&to.path));
        if first {
            self.paths.insert(0, path_entry);
            self.doc_texts.insert(0, doc_entry);
        } else {
            self.paths.push(path_entry);
            self.doc_texts.push(doc_entry);
        }

        if from.path.leading_colon.is_none()
            && to.path.leading_colon.is_none()
            && from.path.segments.len() == 1
            && to.path.segments.len() == 1
            && from.path.segments[0].arguments.is_none()
            && to.path.segments[0].arguments.is_none()
        {
            let ident_entry = (
                from.path.segments[0].ident.to_string(),
                to.path.segments[0].ident.clone(),
            );
            if first {
                self.idents.insert(0, ident_entry);
            } else {
                self.idents.push(ident_entry);
            }
        }
    }

    /// Register the annotated type/trait's exact output rename. These entries
    /// take precedence over user substitutions so self-references follow the
    /// item name selected for this variant.
    pub fn add_item_type(&mut self, from: &Ident, to: &Ident) {
        if from == to {
            return;
        }
        let from: Type = syn::parse_quote!(#from);
        let to: Type = syn::parse_quote!(#to);
        self.register_type(&from, &to, true);
    }

    /// Register an exact output rename for a free function so recursive calls
    /// and references to the source name follow the generated item name.
    pub fn add_item_call(&mut self, from: &Ident, to: &Ident) {
        if from != to {
            self.calls.insert(0, (from.to_string(), to.clone()));
            self.doc_texts.insert(0, (from.to_string(), to.to_string()));
        }
    }

    /// Register an automatic method rename (from renaming the methods of a
    /// two-variant trait/impl) so that calls in bodies follow the rename.
    pub fn add_auto_call(&mut self, from: &Ident, to: &Ident) {
        // User-provided `sync(calls(...))` entries were added first and win.
        self.calls.push((from.to_string(), to.clone()));
        if from != to {
            self.doc_texts.push((from.to_string(), to.to_string()));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
            && self.paths.is_empty()
            && self.idents.is_empty()
            && self.calls.is_empty()
    }

    pub fn lookup_call(&self, name: &Ident) -> Option<&Ident> {
        self.lookup_call_str(&name.to_string())
    }

    fn lookup_call_str(&self, name: &str) -> Option<&Ident> {
        self.calls
            .iter()
            .find(|(from, _)| from == name)
            .map(|(_, to)| to)
    }

    /// Whole-type match: returns the replacement if `ty` matches an entry
    /// token-for-token.
    pub fn subst_type(&self, ty: &Type) -> Option<Type> {
        if self.types.is_empty() {
            return None;
        }
        let key = ty.to_token_stream().to_string();
        self.types
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, t)| t.clone())
    }

    /// Prefix-substitute a path in place (`AsyncStore::new` → `Store::new`).
    /// In expression context, generic arguments on substituted segments get a
    /// turbofish (`Foo::<T>`) so the result parses as an expression.
    pub fn subst_path(&self, path: &mut Path, expr_ctx: bool) -> bool {
        for (src, dst) in &self.paths {
            if path.leading_colon.is_some() != src.leading_colon.is_some()
                || path.segments.len() < src.segments.len()
            {
                continue;
            }
            let matches = path
                .segments
                .iter()
                .take(src.segments.len())
                .zip(&src.segments)
                .all(path_segment_matches);
            if !matches {
                continue;
            }

            let mut segments: Punctuated<PathSegment, Token![::]> = Punctuated::new();
            for seg in &dst.segments {
                segments.push(seg.clone());
            }

            // A bare source pattern such as `AsyncTrait => Trait` matches
            // `AsyncTrait<T>` and carries its generic arguments to `Trait<T>`.
            if let (Some(src_last), Some(actual_last), Some(dst_last)) = (
                src.segments.last(),
                path.segments.iter().nth(src.segments.len() - 1),
                segments.last_mut(),
            ) && src_last.arguments.is_none()
                && dst_last.arguments.is_none()
            {
                dst_last.arguments = actual_last.arguments.clone();
            }

            for seg in path.segments.iter().skip(src.segments.len()) {
                segments.push(seg.clone());
            }
            if expr_ctx {
                for seg in &mut segments {
                    if let PathArguments::AngleBracketed(args) = &mut seg.arguments {
                        args.colon2_token = Some(<Token![::]>::default());
                    }
                }
            }
            path.leading_colon = dst.leading_colon;
            path.segments = segments;
            return true;
        }
        false
    }

    /// Best-effort rewriting inside macro token streams for the sync
    /// expansion: single-ident type substitutions, and call renames
    /// recognized as `name ( ... )`.
    pub fn rewrite_macro_tokens(&self, tokens: TokenStream) -> TokenStream {
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        let mut out: Vec<TokenTree> = Vec::with_capacity(trees.len());
        let mut i = 0;
        while i < trees.len() {
            match &trees[i] {
                TokenTree::Group(group) => {
                    let mut new =
                        Group::new(group.delimiter(), self.rewrite_macro_tokens(group.stream()));
                    new.set_span(group.span());
                    out.push(TokenTree::Group(new));
                    i += 1;
                }
                TokenTree::Ident(ident) => {
                    let name = ident.to_string();
                    if let Some(to) = self.lookup_call_str(&name)
                        && let Some(TokenTree::Group(args)) = trees.get(i + 1)
                        && args.delimiter() == Delimiter::Parenthesis
                    {
                        out.push(TokenTree::Ident(Ident::new(&to.to_string(), ident.span())));
                        let mut new_args = Group::new(
                            Delimiter::Parenthesis,
                            self.rewrite_macro_tokens(args.stream()),
                        );
                        new_args.set_span(args.span());
                        out.push(TokenTree::Group(new_args));
                        i += 2;
                        continue;
                    }
                    if let Some((_, to)) = self.idents.iter().find(|(from, _)| *from == name) {
                        out.push(TokenTree::Ident(Ident::new(&to.to_string(), ident.span())));
                    } else {
                        out.push(trees[i].clone());
                    }
                    i += 1;
                }
                _ => {
                    out.push(trees[i].clone());
                    i += 1;
                }
            }
        }
        out.into_iter().collect()
    }

    /// Substitute the table's spellings inside doc comment text, matching at
    /// identifier boundaries just like the code rewrites: `paths` entries
    /// (including module-qualified ones) and `calls` renames. A match must not
    /// be preceded by an identifier character or `::` (a longer path), and
    /// must not be followed by an identifier character; a trailing `::` is
    /// fine, mirroring path prefix substitution (`AsyncStore::new`).
    pub fn rewrite_doc_text(&self, text: &str) -> Option<String> {
        if self.doc_texts.is_empty() {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        let mut prev: Option<char> = None;
        let mut changed = false;
        'outer: while !rest.is_empty() {
            let boundary = prev.is_none_or(|c| !is_ident_char(c) && c != ':');
            if boundary {
                for (from, to) in &self.doc_texts {
                    if rest.starts_with(from.as_str())
                        && rest[from.len()..]
                            .chars()
                            .next()
                            .is_none_or(|c| !is_ident_char(c))
                    {
                        out.push_str(to);
                        rest = &rest[from.len()..];
                        prev = from.chars().next_back();
                        changed = true;
                        continue 'outer;
                    }
                }
            }
            let ch = rest.chars().next().expect("rest is non-empty");
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
            prev = Some(ch);
        }
        changed.then_some(out)
    }
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// A path's doc-comment spelling: its token string with whitespace removed
/// (`mod_x :: AsyncThing` → `mod_x::AsyncThing`).
fn path_doc_text(path: &Path) -> String {
    path.to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

fn path_segment_matches((actual, pattern): (&PathSegment, &PathSegment)) -> bool {
    if actual.ident != pattern.ident {
        return false;
    }
    pattern.arguments.is_none()
        || path_arguments_key(&actual.arguments) == path_arguments_key(&pattern.arguments)
}

fn path_arguments_key(arguments: &PathArguments) -> String {
    let mut arguments = arguments.clone();
    if let PathArguments::AngleBracketed(args) = &mut arguments {
        args.colon2_token = None;
    }
    arguments.to_token_stream().to_string()
}

/// Best-effort `.await` stripping inside macro token streams for the sync
/// expansion (e.g. `assert_eq!(x.get().await, 1)`).
pub fn strip_awaits_in_tokens(tokens: TokenStream) -> TokenStream {
    let mut out: Vec<TokenTree> = Vec::new();
    let mut iter = tokens.into_iter().peekable();
    while let Some(tree) = iter.next() {
        match tree {
            TokenTree::Group(group) => {
                let mut new = Group::new(group.delimiter(), strip_awaits_in_tokens(group.stream()));
                new.set_span(group.span());
                out.push(TokenTree::Group(new));
            }
            TokenTree::Punct(punct) if punct.as_char() == '.' => {
                if let Some(TokenTree::Ident(next)) = iter.peek()
                    && next == "await"
                {
                    iter.next();
                } else {
                    out.push(TokenTree::Punct(punct));
                }
            }
            other => out.push(other),
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::SubstTable;
    use syn::Type;

    fn table(from: &str, to: &str) -> SubstTable {
        let mut table = SubstTable::default();
        let from: Type = syn::parse_str(from).unwrap();
        let to: Type = syn::parse_str(to).unwrap();
        table.register_type(&from, &to, false);
        table
    }

    #[test]
    fn doc_text_matches_at_identifier_boundaries() {
        let table = table("AsyncStore", "Store");
        assert_eq!(
            table.rewrite_doc_text("[`AsyncStore`], AsyncStore::new, Vec<AsyncStore>"),
            Some("[`Store`], Store::new, Vec<Store>".into())
        );
        // No partial identifiers, no mid-path matches.
        assert_eq!(table.rewrite_doc_text("AsyncStoreExt"), None);
        assert_eq!(table.rewrite_doc_text("MyAsyncStore"), None);
        assert_eq!(table.rewrite_doc_text("other::AsyncStore"), None);
    }

    #[test]
    fn doc_text_matches_module_qualified_paths() {
        let table = table("mod_x::AsyncThing", "mod_y::Thing");
        assert_eq!(
            table.rewrite_doc_text("see [`mod_x::AsyncThing`] and mod_x::AsyncThing::new"),
            Some("see [`mod_y::Thing`] and mod_y::Thing::new".into())
        );
        assert_eq!(table.rewrite_doc_text("outer::mod_x::AsyncThing"), None);
    }
}
