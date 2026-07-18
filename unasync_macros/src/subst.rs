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
}

impl SubstTable {
    pub fn build(cfg: &Config) -> Self {
        let mut table = SubstTable::default();
        for entry in &cfg.types {
            table.register_type(&entry.async_, &entry.sync, false);
        }
        for swap in &cfg.calls {
            table
                .calls
                .push((swap.async_.to_string(), swap.sync.clone()));
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
        if first {
            self.paths.insert(0, path_entry);
        } else {
            self.paths.push(path_entry);
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
        }
    }

    /// Register an automatic method rename (from renaming the methods of a
    /// dual trait/impl) so that calls in bodies follow the rename.
    pub fn add_auto_call(&mut self, from: &Ident, to: &Ident) {
        // User-provided `calls(...)` entries were added first and win.
        self.calls.push((from.to_string(), to.clone()));
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
