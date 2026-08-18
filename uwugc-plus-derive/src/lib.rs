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

    let mut slices = Vec::new();
    for field in fields {
        let field_type = &field.ty;
        where_clause.predicates.push(syn::parse_quote!{
            #field_type: ::uwugc_plus::HasDescriptor
        });

        slices.push(quote! {
            match &<#field_type as ::uwugc_plus::HasDescriptor>::DESCRIPTOR.fields {
                ::std::borrow::Cow::Borrowed(slice) => slice,
                ::std::borrow::Cow::Owned(_) => unreachable!(),
            }
        });
    }

    let aligned_8_error = format!("'{name}' must be aligned to 8 bytes");
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    TokenStream::from(quote! {
        unsafe impl #impl_generics ::uwugc_plus::HasDescriptor for #name #type_generics #where_clause {
            const DESCRIPTOR: &'static ::uwugc_plus::Descriptor = &unsafe { ::uwugc_plus::Descriptor::new(
                ::std::borrow::Cow::Borrowed({
                    const SLICES: &[&[usize]] = &[ #(#slices),* ];
                    const TOTAL_LEN: usize = {
                        let mut current = 0;
                        let mut idx = 0;
                        while idx < SLICES.len() {
                            current += SLICES[idx].len();
                            idx += 1;
                        }
                        current
                    };

                    // HasDescriptor requires struct to be aligned to at most 8 bytes
                    const _: () = assert!(::std::mem::align_of::<#name>() <= 8, #aligned_8_error);

                    const TEMP: [usize; TOTAL_LEN] = {
                        let mut result = [0; TOTAL_LEN];
                        let mut i = 0;
                        let mut out_idx = 0;
                        while i < SLICES.len() {
                            let mut j = 0;
                            let cur = SLICES[i];

                            while j < cur.len() {
                                result[out_idx] = cur[j];
                                out_idx += 1;
                                j += 1;
                            }

                            i += 1;
                        }

                        result
                    };

                    &TEMP
                }),
                ::std::mem::size_of::<Self>(),
            ) };
        }
    })
}


