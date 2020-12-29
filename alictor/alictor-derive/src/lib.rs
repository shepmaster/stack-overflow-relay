//! https://draft.ryhl.io/blog/actors-with-tokio/

use quote::{format_ident, quote};
use syn::parse_macro_input;

#[proc_macro_attribute]
pub fn alictor(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let options = parse_macro_input!(attr as RawOptions);
    let inherent_impl = parse_macro_input!(item as syn::ItemImpl);

    let mut blocking = None;
    for option in options.0 {
        match option {
            RawOption::Kind { kind, .. } => {
                assert!(blocking.is_none(), "Must only set one kind");
                blocking = Some(matches!(kind, RawKind::Blocking { .. }));
            }
        }
    }

    const DEFAULT_BLOCKING: bool = false;
    let blocking = blocking.unwrap_or(DEFAULT_BLOCKING);

    let ty = match &*inherent_impl.self_ty {
        syn::Type::Path(p) => p,
        _ => panic!("TODO: Only paths"),
    };
    let ty = ty.path.get_ident().expect("TODO: Only locals");

    struct Method<'a> {
        name: &'a syn::Ident,
        ret_ty: Option<&'a syn::Type>,
        arg_tys: Vec<&'a syn::Type>,
    }

    impl Method<'_> {
        fn ret_ty(&self) -> proc_macro2::TokenStream {
            self.ret_ty.map_or_else(|| quote! { () }, |t| quote! { #t })
        }

        fn arg_names(&self) -> Vec<syn::Ident> {
            (0u32..)
                .map(|i| format_ident!("a{}", i))
                .take(self.arg_tys.len())
                .collect()
        }
    }

    let mut methods = vec![];
    for item in &inherent_impl.items {
        let m = match item {
            syn::ImplItem::Method(m) => m,
            _ => continue,
        };

        let name = &m.sig.ident;
        let ret_ty = match &m.sig.output {
            syn::ReturnType::Default => None,
            syn::ReturnType::Type(_, t) => Some(&**t),
        };

        let mut inputs = m.sig.inputs.iter();
        let first = inputs.next();
        if !matches!(first, Some(syn::FnArg::Receiver(_))) {
            panic!("TODO: Must be a method");
        }

        let mut arg_tys = vec![];
        for arg in inputs {
            let arg = match arg {
                syn::FnArg::Typed(a) => a,
                _ => panic!("TODO: What is this function"),
            };

            arg_tys.push(&*arg.ty);
        }

        methods.push(Method {
            name,
            ret_ty,
            arg_tys,
        })
    }

    // ----------

    let command_enum_variants = methods.iter().map(|m| {
        let Method { name, arg_tys, .. } = m;
        let ret_ty = m.ret_ty();

        quote! {
            #name(alictor::reexport::futures::channel::oneshot::Sender<#ret_ty>, #(#arg_tys),*)
        }
    });

    let command_enum_name = format_ident!("{}Command", ty);
    let command_enum = quote! {
        #[derive(Debug)]
        #[allow(non_camel_case_types)]
        enum #command_enum_name {
            #(#command_enum_variants),*
        }
    };

    // ----------

    let handle_methods = methods.iter().map(|m| {
        let Method { name, arg_tys, .. } = m;
        let ret_ty = m.ret_ty();
        let arg_names = m.arg_names();

        let try_name = format_ident!("try_{}", name);
        let args: Vec<_> = arg_names.iter().zip(arg_tys).map(|(n, ty)| quote! { #n: #ty }).collect();

        quote! {
            pub async fn #try_name(&mut self, #(#args),*) -> Result<#ret_ty, alictor::ActorError> {
                let (tx, rx) = alictor::reexport::futures::channel::oneshot::channel();

                // Ignore send errors. If this send fails, so does the
                // rx.await below. There's no reason to check for the
                // same failure twice.
                let _ = alictor::reexport::futures::SinkExt::send(&mut self.0, #command_enum_name::#name(tx, #(#arg_names),*)).await;
                alictor::reexport::snafu::ResultExt::context(rx.await, alictor::ActorContext)
            }

            pub async fn #name(&mut self, #(#args),*) -> #ret_ty {
                self.#try_name(#(#arg_names),*).await.expect("Actor is no longer running")
            }
        }
    });

    let handle_name = format_ident!("{}Handle", ty);
    let handle = quote! {
        #[derive(Debug, Clone)]
        pub struct #handle_name(alictor::reexport::futures::channel::mpsc::Sender<#command_enum_name>);

        impl #handle_name {
            #(#handle_methods)*
        }
    };

    // ----------

    let command_enum_variants = methods.iter().map(|m| {
        let Method { name, .. } = m;
        let arg_names = m.arg_names();

        quote! {
            #command_enum_name::#name(__r, #(#arg_names),*) => {
                let retval = self.#name(#(#arg_names),*);

                // If we couldn't respond, that's OK
                let _ = __r.send(retval);
            }
        }
    });

    let dispatch = quote! {
        match cmd {
            #(#command_enum_variants)*
        }
    };

    let spawned_task = if blocking {
        quote! {
            alictor::reexport::tokio::task::spawn_blocking(move || {
                let mut rx = alictor::reexport::futures::executor::block_on_stream(rx);
                while let Some(cmd) = rx.next() {
                    #dispatch
                }
            })
        }
    } else {
        quote! {
            alictor::reexport::tokio::task::spawn(async move {
                let mut rx = rx;
                while let Some(cmd) = alictor::reexport::futures::StreamExt::next(&mut rx).await {
                    #dispatch
                }
            })
        }
    };

    let inherent_impl_spawn = quote! {
        impl #ty {
            pub fn spawn(#[allow(unused_mut)] mut self) -> (#handle_name, alictor::reexport::tokio::task::JoinHandle<()>) {
                let (tx, rx) = alictor::reexport::futures::channel::mpsc::channel(10);
                let child = #spawned_task;
                (#handle_name(tx), child)
            }
        }
    };

    // ----------

    (quote! {
        #inherent_impl
        #inherent_impl_spawn

        #command_enum

        #handle
    })
    .into()
}

mod kw {
    syn::custom_keyword!(kind);
    syn::custom_keyword!(blocking);
}

struct RawOptions(syn::punctuated::Punctuated<RawOption, syn::token::Comma>);

impl syn::parse::Parse for RawOptions {
    fn parse(input: syn::parse::ParseStream<'_>) -> Result<Self, syn::Error> {
        syn::punctuated::Punctuated::parse_terminated(input).map(Self)
    }
}

enum RawOption {
    Kind {
        #[allow(unused)]
        kind_token: kw::kind,
        #[allow(unused)]
        eq_token: syn::token::Eq,
        kind: RawKind,
    },
}

impl syn::parse::Parse for RawOption {
    fn parse(input: syn::parse::ParseStream<'_>) -> Result<Self, syn::Error> {
        let lookahead = input.lookahead1();
        if lookahead.peek(kw::kind) {
            Ok(Self::Kind {
                kind_token: input.parse()?,
                eq_token: input.parse()?,
                kind: input.parse()?,
            })
        } else {
            Err(lookahead.error())
        }
    }
}

enum RawKind {
    Async {
        #[allow(unused)]
        async_token: syn::token::Async,
    },
    Blocking {
        #[allow(unused)]
        blocking_token: kw::blocking,
    },
}

impl syn::parse::Parse for RawKind {
    fn parse(input: syn::parse::ParseStream<'_>) -> Result<Self, syn::Error> {
        let lookahead = input.lookahead1();
        if lookahead.peek(syn::token::Async) {
            Ok(Self::Async {
                async_token: input.parse()?,
            })
        } else if lookahead.peek(kw::blocking) {
            Ok(Self::Blocking {
                blocking_token: input.parse()?,
            })
        } else {
            Err(lookahead.error())
        }
    }
}
