use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn main(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}
