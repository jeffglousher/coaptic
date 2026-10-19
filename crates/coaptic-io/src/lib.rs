//! Explicit, local transformation of shared protocol bodies.
//!
//! The sync item is emitted unchanged. Only declared call sites and the
//! `DatagramIo` bound change in its async twin; this does not drive an executor,
//! allocate protocol memory, or manufacture transport completion.

use proc_macro::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{Expr, Ident, ItemFn, Path, Token, parenthesized, parse_macro_input};

struct Mapping {
    from: Path,
    to: Path,
}

impl Parse for Mapping {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            from: input.parse()?,
            to: {
                input.parse::<Token![=>]>()?;
                input.parse()?
            },
        })
    }
}

struct Options {
    name: Ident,
    calls: Vec<Mapping>,
    methods: Vec<Mapping>,
    bound: Option<Path>,
    renames: Vec<Mapping>,
    visibility: Option<syn::Visibility>,
}

impl Parse for Options {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        let mut result = Self {
            name,
            calls: Vec::new(),
            methods: Vec::new(),
            bound: None,
            renames: Vec::new(),
            visibility: None,
        };
        while !input.is_empty() {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let section: Ident = input.parse()?;
            let content;
            parenthesized!(content in input);
            if section == "visibility" {
                result.visibility = Some(content.parse()?);
                continue;
            }
            if section == "bound" {
                result.bound = Some(content.parse()?);
                continue;
            }
            let entries = Punctuated::<Mapping, Token![,]>::parse_terminated(&content)?;
            match section.to_string().as_str() {
                "calls" => result.calls.extend(entries),
                "renames" => result.renames.extend(entries),
                "methods" => {
                    for entry in entries {
                        if entry.from.get_ident().is_none() || entry.to.get_ident().is_none() {
                            return Err(syn::Error::new_spanned(
                                section,
                                "methods require bare identifiers",
                            ));
                        }
                        result.methods.push(entry);
                    }
                }
                _ => {
                    return Err(syn::Error::new_spanned(
                        section,
                        "expected calls(...) or methods(...)",
                    ));
                }
            }
        }
        Ok(result)
    }
}

struct Transform<'a> {
    options: &'a Options,
}

// Turbofish arguments are call-site types, not part of the declared mapping.
// Preserve them on the destination rather than silently missing generic calls.
fn mapped_path(path: &Path, mappings: &[Mapping]) -> Option<Path> {
    let mapping = mappings.iter().find(|mapping| {
        mapping.from.leading_colon == path.leading_colon
            && mapping.from.segments.len() == path.segments.len()
            && mapping
                .from
                .segments
                .iter()
                .zip(&path.segments)
                .all(|(from, actual)| from.ident == actual.ident)
    })?;
    let mut result = mapping.to.clone();
    if let (Some(destination), Some(source)) = (result.segments.last_mut(), path.segments.last()) {
        destination.arguments = source.arguments.clone();
    }
    Some(result)
}

impl VisitMut for Transform<'_> {
    fn visit_trait_bound_mut(&mut self, bound: &mut syn::TraitBound) {
        visit_mut::visit_trait_bound_mut(self, bound);
        if let Some(segment) = bound.path.segments.last_mut()
            && segment.ident == "DatagramIo"
        {
            if let Some(replacement) = &self.options.bound {
                bound.path = replacement.clone();
            } else {
                segment.ident = Ident::new("AsyncDatagramIo", segment.ident.span());
            }
        }
    }

    fn visit_expr_mut(&mut self, expr: &mut Expr) {
        visit_mut::visit_expr_mut(self, expr);
        let matched = match expr {
            Expr::Call(call) => match &mut *call.func {
                Expr::Path(path) => {
                    if let Some(destination) = mapped_path(&path.path, &self.options.calls) {
                        path.path = destination;
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            },
            Expr::MethodCall(call) => {
                if let Some(mapping) = self
                    .options
                    .renames
                    .iter()
                    .find(|m| m.from.is_ident(&call.method))
                {
                    call.method = mapping.to.get_ident().expect("bare method rename").clone();
                    return;
                }
                if let Some(mapping) = self
                    .options
                    .methods
                    .iter()
                    .find(|m| m.from.is_ident(&call.method))
                {
                    call.method = mapping.to.get_ident().expect("validated method").clone();
                    true
                } else {
                    false
                }
            }
            _ => false,
        };
        if matched {
            *expr = syn::parse_quote! { (#expr).await };
        }
    }
}

/// Emit an async twin using only explicitly declared suspension sites.
///
/// `#[dual_io(foo_async, calls(bar => bar_async), methods(send_tx => send_tx_async))]`
/// preserves `foo` and emits `foo_async`. The compiler checks every resulting
/// await, including rejection of awaits inside synchronous closures. Ownership
/// and cancellation remain the responsibility of the protocol implementation.
#[proc_macro_attribute]
pub fn dual_io(args: TokenStream, input: TokenStream) -> TokenStream {
    let options = parse_macro_input!(args as Options);
    let original = parse_macro_input!(input as ItemFn);
    if original.sig.asyncness.is_some() {
        return syn::Error::new_spanned(
            &original.sig,
            "dual_io requires a synchronous source function",
        )
        .to_compile_error()
        .into();
    }
    let mut asynchronous = original.clone();
    asynchronous.sig.ident = options.name.clone();
    asynchronous.sig.asyncness = Some(Default::default());
    if let Some(visibility) = &options.visibility {
        asynchronous.vis = visibility.clone();
    }
    Transform { options: &options }.visit_item_fn_mut(&mut asynchronous);
    quote! { #original #asynchronous }.into()
}
