use proc_macro2::Span;
use syn::{spanned::Spanned, Fields, Item};

pub(crate) fn derive_debug_with_db(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let item = syn::parse_macro_input!(input as Item);
    let res = match item {
        syn::Item::Struct(item) => derive_for_struct(item),
        syn::Item::Enum(item) => derive_for_enum(item),
        _ => Err(syn::Error::new(
            item.span(),
            "derive(DebugWithDb) can only be applied to structs",
        )),
    };
    match res {
        Ok(s) => s.into(),
        Err(err) => err.into_compile_error().into(),
    }
}

fn add_debug(generics: &syn::Generics) -> syn::Generics {
    let db_type = quote! { <crate::Jar as ::salsa::jar::Jar<'db>>::DynDb };

    let mut generics_with_debug: syn::Generics = parse_quote! { <'db> };
    generics_with_debug
        .params
        .extend(generics.params.iter().cloned());
    for param in generics_with_debug.type_params_mut() {
        param
            .bounds
            .push_value(parse_quote! { ::salsa::DebugWithDb<#db_type> });
    }
    generics_with_debug
}

pub(crate) fn derive_for_struct(
    struct_item: syn::ItemStruct,
) -> syn::Result<proc_macro2::TokenStream> {
    let ident = struct_item.ident;

    let generics = struct_item.generics;

    let db_type = quote! { <crate::Jar as ::salsa::jar::Jar<'db>>::DynDb };

    let generics_with_debug = add_debug(&generics);

    let ident_string = ident.to_string();

    let fields = debug_fields(
        &ident_string,
        |index, field| match &field.ident {
            Some(ident) => {
                quote! { self.#ident }
            }
            None => {
                let index = syn::Index::from(index);
                quote! { self.#index }
            }
        },
        &struct_item.fields,
    );

    Ok(quote_spanned! { ident.span() =>
        impl #generics_with_debug ::salsa::DebugWithDb<#db_type> for #ident #generics {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>, _db: &#db_type, _include_all_fields: bool) -> ::std::fmt::Result {
                #[allow(unused_imports)]
                use ::salsa::debug::helper::Fallback;
                #fields
            }
        }
    })
}

fn debug_fields(
    name: &str,
    accessor: impl Fn(usize, &syn::Field) -> proc_macro2::TokenStream,
    fields: &syn::Fields,
) -> proc_macro2::TokenStream {
    use proc_macro2::TokenStream;

    fn make_fields_debug<'a>(
        accessor: impl Fn(usize, &syn::Field) -> proc_macro2::TokenStream,
        fields: impl Iterator<Item = &'a syn::Field>,
    ) -> TokenStream {
        let db_type = quote! { <crate::Jar as salsa::jar::Jar<'db>>::DynDb };
        fields
            .enumerate()
            .map(|(index, ref field)| -> TokenStream {
                let field_name_arg = match &field.ident {
                    Some(x) => {
                        let id = x.to_string();
                        quote! { #id, }
                    }
                    None => quote! {},
                };
                let field_getter = accessor(index, &field);
                let field_ty = &field.ty;

                quote_spanned! { field.span() =>
                    debug_struct = debug_struct.field(
                        #field_name_arg
                        &::salsa::debug::helper::SalsaDebug::<#field_ty, #db_type>::salsa_debug(
                            #[allow(clippy::needless_borrow)]
                            &#field_getter,
                            _db,
                            _include_all_fields
                        )
                    );
                }
            })
            .collect::<TokenStream>()
    }
    match fields {
        Fields::Unit => quote! {},
        Fields::Named(ref fields) => {
            let fields_debug = make_fields_debug(accessor, fields.named.iter());
            quote_spanned! { fields.span() =>
                let mut debug_struct = &mut f.debug_struct(#name);
                #fields_debug
                debug_struct.finish()
            }
        }
        Fields::Unnamed(ref fields) => {
            let fields_debug = make_fields_debug(accessor, fields.unnamed.iter());
            quote_spanned! { fields.span() =>
                let mut debug_struct = &mut f.debug_tuple(#name);
                #fields_debug
                debug_struct.finish()
            }
        }
    }
}

pub(crate) fn derive_for_enum(enum_item: syn::ItemEnum) -> syn::Result<proc_macro2::TokenStream> {
    use proc_macro2::TokenStream;

    let ident = enum_item.ident;
    let generics = enum_item.generics;

    let db_type = quote! { <crate::Jar as ::salsa::jar::Jar<'db>>::DynDb };

    let generics_with_debug = add_debug(&generics);

    let variants = enum_item
        .variants
        .iter()
        .map(|variant| {
            let ident = &variant.ident;
            let ident_str = ident.to_string();
            match variant.fields {
                Fields::Unit => quote! {
                    Self::#ident => f.debug_tuple(#ident_str).finish(),
                },
                Fields::Named(ref fields) => {
                    let patterns = fields
                        .named
                        .iter()
                        .map(|field| {
                            let ident = &field.ident;
                            quote! { #ident , }
                        })
                        .collect::<TokenStream>();
                    let fields = debug_fields(
                        &ident_str,
                        |_, field| {
                            let ident = &field.ident;
                            quote! { #ident }
                        },
                        &variant.fields,
                    );
                    quote! {
                        Self::#ident{#patterns} => { #fields },
                    }
                }
                Fields::Unnamed(ref fields) => {
                    let patterns = fields
                        .unnamed
                        .iter()
                        .enumerate()
                        .map(|(index, _)| {
                            let ident =
                                proc_macro2::Ident::new(&format!("x{}", index), Span::call_site());
                            quote! { #ident , }
                        })
                        .collect::<TokenStream>();
                    let fields = debug_fields(
                        &ident_str,
                        |index, _| {
                            let ident =
                                proc_macro2::Ident::new(&format!("x{}", index), Span::call_site());
                            quote! { #ident }
                        },
                        &variant.fields,
                    );
                    quote! {
                        Self::#ident(#patterns) => { #fields },
                    }
                }
            }
        })
        .collect::<TokenStream>();

    Ok(quote_spanned! {ident.span()=>
        impl #generics_with_debug ::salsa::DebugWithDb<#db_type> for #ident #generics {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>, _db: &#db_type, _include_all_fields: bool) -> ::std::fmt::Result {
                #[allow(unused_imports)]
                use ::salsa::debug::helper::Fallback;
                match self {
                    #variants
                }
            }
        }
    })
}
