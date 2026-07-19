//! Type/path/call substitution tables applied to the derived sync expansion.
//!
//! The source is written in async form; entries map the async spelling to the sync spelling.
//! Matching is purely syntactic (token-level, after parse normalization).
//! There is no name resolution: aliased spellings must be added to the table by the user.

use proc_macro2::{Delimiter, Group, TokenStream, TokenTree};
use quote::ToTokens;
use syn::punctuated::Punctuated;
use syn::{Ident, Path, PathArguments, PathSegment, Token, Type};

use crate::config::{Config, RenamePattern};

#[derive(Default)]
pub struct SubstTable {
    /// Whole-type substitutions, keyed by the normalized token string of the async (source) spelling.
    types: Vec<(String, Type)>,
    /// Plain-path substitutions (types whose async spelling is a bare path with no generic arguments).
    /// Used for expression paths (`AsyncStore::new()`), struct literals, patterns, trait bounds and trait-impl targets, where the path may extend past the matched prefix.
    paths: Vec<(Path, Path)>,
    /// Single-ident → single-ident subset of `paths`, safe to apply inside macro token streams.
    idents: Vec<(String, Ident)>,
    /// Call renames: async (source) name → sync name.
    calls: Vec<(String, Ident)>,
    /// Pattern entries from `types("Async{}")`, consulted for bare type idents after the exact tables.
    type_patterns: Vec<RenamePattern>,
    /// Pattern entries from `fns("{}_async")`, consulted for call names after the exact `calls` entries.
    call_patterns: Vec<RenamePattern>,
    /// Textual spellings of `paths` and `calls` entries, applied to doc comments at identifier boundaries (`[`AsyncStore`]` → `[`Store`]`).
    doc_texts: Vec<(String, String)>,
}

impl SubstTable {
    pub fn build(cfg: &Config) -> Self {
        let mut table = SubstTable::default();
        for entry in &cfg.sync.types {
            table.register_type(&entry.async_, &entry.sync, false);
        }
        for rename in &cfg.sync.fns {
            table
                .calls
                .push((rename.async_.to_string(), rename.sync.clone()));
            table
                .doc_texts
                .push((rename.async_.to_string(), rename.sync.to_string()));
        }
        table.type_patterns = cfg.sync.type_patterns.clone();
        table.call_patterns = cfg.sync.fn_patterns.clone();
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

    /// Register the annotated type/trait's exact output rename.
    /// These entries take precedence over user substitutions so self-references follow the item name selected for this variant.
    pub fn add_item_type(&mut self, from: &Ident, to: &Ident) {
        if from == to {
            return;
        }
        let from: Type = syn::parse_quote!(#from);
        let to: Type = syn::parse_quote!(#to);
        self.register_type(&from, &to, true);
    }

    /// Register an exact output rename for a free function so recursive calls and references to the source name follow the generated item name.
    pub fn add_item_call(&mut self, from: &Ident, to: &Ident) {
        if from != to {
            self.calls.insert(0, (from.to_string(), to.clone()));
            self.doc_texts.insert(0, (from.to_string(), to.to_string()));
        }
    }

    /// Register an automatic method rename (from renaming the methods of a two-variant trait/impl) so that calls in bodies follow the rename.
    pub fn add_auto_call(&mut self, from: &Ident, to: &Ident) {
        // User-provided `sync(fns(...))` entries were added first and win.
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
            && self.type_patterns.is_empty()
            && self.call_patterns.is_empty()
    }

    pub fn lookup_call(&self, name: &Ident) -> Option<Ident> {
        self.lookup_call_str(&name.to_string())
            .map(|to| Ident::new(&to, name.span()))
    }

    fn lookup_call_str(&self, name: &str) -> Option<String> {
        self.calls
            .iter()
            .find(|(from, _)| from == name)
            .map(|(_, to)| to.to_string())
            .or_else(|| pattern_rename(&self.call_patterns, name))
    }

    /// Whole-type match: returns the replacement if `ty` matches an entry token-for-token.
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

    /// Pattern fallback for type position: rename the leading ident of a plain path type in place (`AsyncGroup<T>` → `Group<T>` under `types("Async{}")`).
    /// Generic arguments are untouched; the caller keeps visiting them.
    pub fn subst_type_pattern(&self, ty: &mut Type) -> bool {
        match ty {
            Type::Path(type_path) if type_path.qself.is_none() => {
                self.pattern_rename_leading_segment(&mut type_path.path)
            }
            _ => false,
        }
    }

    /// Rename a path's leading segment through `type_patterns`, mirroring how exact single-ident entries match a path prefix.
    fn pattern_rename_leading_segment(&self, path: &mut Path) -> bool {
        if path.leading_colon.is_some() {
            return false;
        }
        let Some(first) = path.segments.first_mut() else {
            return false;
        };
        let Some(to) = pattern_rename(&self.type_patterns, &first.ident.to_string()) else {
            return false;
        };
        first.ident = Ident::new(&to, first.ident.span());
        true
    }

    /// Prefix-substitute a path in place (`AsyncStore::new` → `Store::new`).
    /// In expression context, generic arguments on substituted segments get a turbofish (`Foo::<T>`) so the result parses as an expression.
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

            // A bare source pattern such as `AsyncTrait => Trait` matches `AsyncTrait<T>` and carries its generic arguments to `Trait<T>`.
            if let (Some(src_last), Some(actual_last), Some(dst_last)) = (
                src.segments.last(),
                path.segments.iter().nth(src.segments.len() - 1),
                segments.last_mut(),
            ) {
                if src_last.arguments.is_none() && dst_last.arguments.is_none() {
                    dst_last.arguments = actual_last.arguments.clone();
                }
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
        // Pattern fallback: the rename is in place, so the segments' own argument
        // spellings (already valid for this position) are preserved as written.
        self.pattern_rename_leading_segment(path)
    }

    /// Best-effort rewriting inside macro token streams for the sync expansion: single-ident type substitutions, and call renames recognized as `name ( ... )` or turbofish `name :: < ... > ( ... )`.
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
                    if let Some(to) = self.lookup_call_str(&name) {
                        if call_follows(&trees, i + 1) {
                            out.push(TokenTree::Ident(Ident::new(&to, ident.span())));
                            i += 1;
                            continue;
                        }
                    }
                    let type_subst = self
                        .idents
                        .iter()
                        .find(|(from, _)| *from == name)
                        .map(|(_, to)| to.to_string())
                        .or_else(|| pattern_rename(&self.type_patterns, &name));
                    if let Some(to) = type_subst {
                        out.push(TokenTree::Ident(Ident::new(&to, ident.span())));
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

    /// Substitute the table's spellings inside doc comment text, matching at identifier boundaries just like the code rewrites: `paths` entries (including module-qualified ones) and `calls` renames.
    /// A match must not be preceded by an identifier character or `::` (a longer path), and must not be followed by an identifier character; a trailing `::` is fine, mirroring path prefix substitution (`AsyncStore::new`).
    pub fn rewrite_doc_text(&self, text: &str) -> Option<String> {
        if self.doc_texts.is_empty()
            && self.type_patterns.is_empty()
            && self.call_patterns.is_empty()
        {
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
                // Pattern fallback: match the whole identifier at this position.
                let ident_len: usize = rest
                    .chars()
                    .take_while(|c| is_ident_char(*c))
                    .map(char::len_utf8)
                    .sum();
                if ident_len > 0 {
                    let ident = &rest[..ident_len];
                    if let Some(to) = pattern_rename(&self.type_patterns, ident)
                        .or_else(|| pattern_rename(&self.call_patterns, ident))
                    {
                        out.push_str(&to);
                        rest = &rest[ident_len..];
                        prev = ident.chars().next_back();
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

/// Whether the tokens from `i` start a call's argument spelling: `( ... )` directly, or a turbofish `:: < ... > ( ... )`.
/// Only the `:: <` prefix of a turbofish is checked; the generic arguments and the argument group that must follow are left to the caller's token walk.
fn call_follows(trees: &[TokenTree], i: usize) -> bool {
    match trees.get(i) {
        Some(TokenTree::Group(args)) => args.delimiter() == Delimiter::Parenthesis,
        Some(TokenTree::Punct(p1)) if p1.as_char() == ':' => {
            matches!(trees.get(i + 1), Some(TokenTree::Punct(p2)) if p2.as_char() == ':')
                && matches!(trees.get(i + 2), Some(TokenTree::Punct(p3)) if p3.as_char() == '<')
        }
        _ => false,
    }
}

/// First matching pattern wins; a rename only applies when the derived name is itself a valid identifier
/// (so e.g. `async_{}` never rewrites `async_2` into the non-ident `2`).
fn pattern_rename(patterns: &[RenamePattern], name: &str) -> Option<String> {
    patterns.iter().find_map(|pattern| {
        pattern
            .sync_name_str(name)
            .filter(|to| syn::parse_str::<Ident>(to).is_ok())
    })
}

/// A path's doc-comment spelling: its token string with whitespace removed (`mod_x :: AsyncThing` → `mod_x::AsyncThing`).
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

/// Best-effort `.await` stripping inside macro token streams for the sync expansion (e.g. `assert_eq!(x.get().await, 1)`).
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
                let awaited =
                    matches!(iter.peek(), Some(TokenTree::Ident(next)) if next == "await");
                if awaited {
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
