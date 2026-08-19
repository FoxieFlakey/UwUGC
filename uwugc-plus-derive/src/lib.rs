use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, parse_macro_input};

#[proc_macro_derive(HasDescriptor)]
pub fn derive_has_descriptor(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let fields = match &input.data {
        Data::Struct(data) => &data.fields,
        _ => {
            return syn::Error::new(name.span(), "HasDescriptor can only work on non enum types (this is due in enums some bit patterns can contains invalid  pointers)")
                .to_compile_error()
                .into()
        }
    };

    let mut generics = input.generics.clone();
    let where_clause = generics.make_where_clause();

    let mut unflattened = Vec::new();
    for (idx, field) in fields.iter().enumerate() {
        let field_type = &field.ty;
        where_clause.predicates.push(syn::parse_quote!{
            #field_type: ::uwugc_plus::HasDescriptor
        });

        let name = field.ident
            .as_ref()
            .map(|id| quote! { #id })
            .unwrap_or_else(|| quote! { #idx });
        unflattened.push(quote! {
            (::std::mem::offset_of!(Self, #name), <#field_type as ::uwugc_plus::HasDescriptor>::DESCRIPTOR)
        });
    }

    let aligned_8_error = format!("'{name}' must be aligned to 8 bytes");
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    TokenStream::from(quote! {
        unsafe impl #impl_generics ::uwugc_plus::HasDescriptor for #name #type_generics #where_clause {
            const DESCRIPTOR: &'static ::uwugc_plus::Descriptor = &unsafe { ::uwugc_plus::Descriptor::new_unflattened(
                ::std::borrow::Cow::Borrowed(const {
                    // HasDescriptor requires struct to be aligned to at most 8 bytes
                    assert!(::std::mem::align_of::<Self>() <= 8, #aligned_8_error);
                    &[]
                }),
                ::std::mem::size_of::<Self>(),
                &[ #(#unflattened),* ]
            ) };
        }
    })
}


