use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, parse_quote, ItemFn};

#[proc_macro_attribute]
pub fn faas_handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input = parse_macro_input!(item as ItemFn);
    let original_block = input.block;

    input.block = Box::new(parse_quote!({
        static __FAAS_SDK_TRACING_INIT: ::std::sync::Once = ::std::sync::Once::new();

        __FAAS_SDK_TRACING_INIT.call_once(|| {
            let subscriber = ::tracing_subscriber::fmt()
                .json()
                .with_ansi(false)
                .without_time()
                .with_current_span(false)
                .with_span_list(false)
                .with_writer(::std::io::stdout)
                .finish();

            let _ = ::tracing::subscriber::set_global_default(subscriber);
        });

        #original_block
    }));

    TokenStream::from(quote!(#input))
}
