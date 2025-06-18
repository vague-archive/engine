#![allow(clippy::todo)]

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DataEnum, DataStruct, DeriveInput, Generics, Ident, parse2, spanned::Spanned};

pub fn derive_serialize_impl(input: TokenStream) -> TokenStream {
    let DeriveInput {
        ident,
        data,
        generics,
        ..
    } = parse2(input).unwrap();

    match &data {
        Data::Struct(data_struct) => derive_serialize_struct(&ident, &generics, data_struct),
        Data::Enum(data_enum) => derive_serialize_enum(&ident, &generics, data_enum),
        Data::Union(_) => todo!("unions not yet supported"),
    }
}

fn derive_serialize_struct(
    ident: &Ident,
    generics: &Generics,
    data_struct: &DataStruct,
) -> proc_macro2::TokenStream {
    let generic_type_idents = generics.type_params().map(|param| &param.ident);

    if data_struct.fields.iter().all(|field| field.ident.is_some()) {
        let field_idents = data_struct
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());
        let field_types = data_struct.fields.iter().map(|field| &field.ty);

        quote! {
            impl #generics ::snapshot::Serialize for #ident <#(#generic_type_idents,)*> {
                #[inline]
                fn serialize<W>(&self, serializer: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
                where
                    W: ::snapshot::WriteUninit,
                {
                    #(<#field_types as ::snapshot::Serialize>::serialize(&self.#field_idents, serializer)?;)*
                    Ok(())
                }
            }
        }
    } else if let Some(field) = data_struct.fields.iter().next() {
        assert!(field.ident.is_none(), "unexpected struct field");
        let field_type = &field.ty;

        quote! {
            impl #generics ::snapshot::Serialize for #ident <#(#generic_type_idents,)*> {
                #[inline]
                fn serialize<W>(&self, serializer: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
                where
                    W: ::snapshot::WriteUninit,
                {
                    <#field_type as ::snapshot::Serialize>::serialize(&self.0, serializer)
                }
            }
        }
    } else {
        // Empty struct.

        quote! {
            impl #generics ::snapshot::Serialize for #ident <#(#generic_type_idents,)*> {
                #[inline]
                fn serialize<W>(&self, serializer: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
                where
                    W: ::snapshot::WriteUninit,
                {
                    Ok(())
                }
            }
        }
    }
}

fn derive_serialize_enum(
    ident: &Ident,
    generics: &Generics,
    data_enum: &DataEnum,
) -> proc_macro2::TokenStream {
    let generic_type_idents = generics.type_params().map(|param| &param.ident);

    let variants = data_enum.variants.iter().enumerate().map(|(i, variant)| {
        let i = u8::try_from(i).expect("too many variants");
        let variant_ident = &variant.ident;

        // Check if the variant is a tuple field.
        if variant
            .fields
            .iter()
            .next()
            .is_some_and(|field| field.ident.is_none())
        {
            let field_idents = variant
                .fields
                .iter()
                .enumerate()
                .map(|(i, field)| Ident::new(&((b'a' + i as u8) as char).to_string(), field.span()));
            let field_idents2 = field_idents.clone();
            let field_types = variant.fields.iter().map(|field| &field.ty);

            return quote! {
                #ident::#variant_ident ( #(#field_idents),* ) => {
                    <u8 as ::snapshot::Serialize>::serialize(&#i, serializer)?;
                    #(<#field_types as ::snapshot::Serialize>::serialize(#field_idents2, serializer)?;)*
                    Ok(())
                }
            };
        }

        let field_types = variant.fields.iter().map(|field| &field.ty);
        let field_idents = variant
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());
        let field_idents2 = variant
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());

        quote! {
            #ident::#variant_ident { #(#field_idents),* } => {
                <u8 as ::snapshot::Serialize>::serialize(&#i, serializer)?;
                #(<#field_types as ::snapshot::Serialize>::serialize(#field_idents2, serializer)?;)*
                Ok(())
            }
        }
    });

    quote! {
        impl #generics ::snapshot::Serialize for #ident <#(#generic_type_idents,)*> {
            #[inline]
            fn serialize<W>(&self, serializer: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
            where
                W: ::snapshot::WriteUninit,
            {
                match self {
                    #(#variants,)*
                    _ => unreachable!(),
                }
            }
        }
    }
}

pub fn derive_serialize_mut_impl(input: TokenStream) -> TokenStream {
    let DeriveInput {
        ident,
        data,
        generics,
        ..
    } = parse2(input).unwrap();

    match &data {
        Data::Struct(data_struct) => derive_serialize_mut_struct(&ident, &generics, data_struct),
        Data::Enum(data_enum) => derive_serialize_mut_enum(&ident, &generics, data_enum),
        Data::Union(_) => todo!("unions not yet supported"),
    }
}

fn derive_serialize_mut_struct(
    ident: &Ident,
    generics: &Generics,
    data_struct: &DataStruct,
) -> proc_macro2::TokenStream {
    let generic_type_idents = generics.type_params().map(|param| &param.ident);

    if data_struct.fields.iter().all(|field| field.ident.is_some()) {
        let field_idents = data_struct
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());
        let field_types = data_struct.fields.iter().map(|field| &field.ty);

        quote! {
            impl #generics ::snapshot::SerializeMut for #ident <#(#generic_type_idents,)*> {
                #[inline]
                fn serialize_mut<W>(&mut self, serializer: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
                where
                    W: ::snapshot::WriteUninit,
                {
                    #(<#field_types as ::snapshot::SerializeMut>::serialize_mut(&mut self.#field_idents, serializer)?;)*
                    Ok(())
                }
            }
        }
    } else if let Some(field) = data_struct.fields.iter().next() {
        assert!(field.ident.is_none(), "unexpected struct field");
        let field_type = &field.ty;

        quote! {
            impl #generics ::snapshot::SerializeMut for #ident <#(#generic_type_idents,)*> {
                #[inline]
                fn serialize_mut<W>(&mut self, serializer: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
                where
                    W: ::snapshot::WriteUninit,
                {
                    <#field_type as ::snapshot::SerializeMut>::serialize_mut(&mut self.0, serializer)
                }
            }
        }
    } else {
        // Empty struct.

        quote! {
            impl #generics ::snapshot::SerializeMut for #ident <#(#generic_type_idents,)*> {
                #[inline]
                fn serialize_mut<W>(&mut self, _: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
                where
                    W: ::snapshot::WriteUninit,
                {
                    Ok(())
                }
            }
        }
    }
}

fn derive_serialize_mut_enum(
    ident: &Ident,
    generics: &Generics,
    data_enum: &DataEnum,
) -> proc_macro2::TokenStream {
    let generic_type_idents = generics.type_params().map(|param| &param.ident);

    let variants = data_enum.variants.iter().enumerate().map(|(i, variant)| {
        let i = u8::try_from(i).expect("too many variants");
        let variant_ident = &variant.ident;

        // Check if the variant is a tuple field.
        if variant
            .fields
            .iter()
            .next()
            .is_some_and(|field| field.ident.is_none())
        {
            let field_idents = variant
                .fields
                .iter()
                .enumerate()
                .map(|(i, field)| Ident::new(&((b'a' + i as u8) as char).to_string(), field.span()));
            let field_idents2 = field_idents.clone();
            let field_types = variant.fields.iter().map(|field| &field.ty);

            return quote! {
                #ident::#variant_ident ( #(#field_idents),* ) => {
                    <u8 as ::snapshot::Serialize>::serialize(&#i, serializer)?;
                    #(<#field_types as ::snapshot::SerializeMut>::serialize_mut(#field_idents2, serializer)?;)*
                    Ok(())
                }
            };
        }

        let field_types = variant.fields.iter().map(|field| &field.ty);
        let field_idents = variant
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());
        let field_idents2 = variant
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());

        quote! {
            #ident::#variant_ident { #(#field_idents),* } => {
                <u8 as ::snapshot::Serialize>::serialize(&#i, serializer)?;
                #(<#field_types as ::snapshot::SerializeMut>::serialize_mut(#field_idents2, serializer)?;)*
                Ok(())
            }
        }
    });

    quote! {
        impl #generics ::snapshot::SerializeMut for #ident <#(#generic_type_idents,)*> {
            #[inline]
            fn serialize_mut<W>(&mut self, serializer: &mut ::snapshot::Serializer<W>) -> ::snapshot::Result<()>
            where
                W: ::snapshot::WriteUninit,
            {
                match self {
                    #(#variants,)*
                    _ => unreachable!(),
                }
            }
        }
    }
}

pub fn derive_deserialize_impl(input: TokenStream) -> TokenStream {
    let DeriveInput {
        ident,
        data,
        generics,
        ..
    } = parse2(input).unwrap();

    match &data {
        Data::Struct(data_struct) => derive_deserialize_struct(&ident, &generics, data_struct),
        Data::Enum(data_enum) => derive_deserialize_enum(&ident, &generics, data_enum),
        Data::Union(_) => todo!("unions not yet supported"),
    }
}

fn derive_deserialize_struct(
    ident: &Ident,
    generics: &Generics,
    data_struct: &DataStruct,
) -> proc_macro2::TokenStream {
    let generic_type_idents = generics.type_params().map(|param| &param.ident);

    if data_struct.fields.iter().all(|field| field.ident.is_some()) {
        let field_idents = data_struct
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());
        let field_types = data_struct.fields.iter().map(|field| &field.ty);

        let field_idents2 = field_idents.clone();
        let field_types2 = field_types.clone();

        quote! {
            impl #generics ::snapshot::Deserialize for #ident <#(#generic_type_idents,)*> {
                #[inline]
                unsafe fn deserialize<R>(deserializer: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<Self>
                where
                    R: ::snapshot::ReadUninit,
                {
                    Ok(Self {
                        #(#field_idents: unsafe {
                            <#field_types as ::snapshot::Deserialize>::deserialize(deserializer)?
                        },)*
                    })
                }

                #[inline]
                unsafe fn deserialize_in_place<R>(&mut self, deserializer: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<()>
                where
                    R: ::snapshot::ReadUninit,
                {
                    #(unsafe {
                        <#field_types2 as ::snapshot::Deserialize>::deserialize_in_place(&mut self.#field_idents2, deserializer)?;
                    })*
                    Ok(())
                }
            }
        }
    } else if let Some(field) = data_struct.fields.iter().next() {
        assert!(field.ident.is_none(), "unexpected struct field");
        let field_type = &field.ty;

        quote! {
            impl #generics ::snapshot::Deserialize for #ident <#(#generic_type_idents,)*> {
                #[inline]
                unsafe fn deserialize<R>(deserializer: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<Self>
                where
                    R: ::snapshot::ReadUninit,
                {
                    Ok(Self( unsafe { <#field_type as ::snapshot::Deserialize>::deserialize(deserializer)? }))
                }

                #[inline]
                unsafe fn deserialize_in_place<R>(&mut self, deserializer: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<()>
                where
                    R: ::snapshot::ReadUninit,
                {
                    unsafe {
                        <#field_type as ::snapshot::Deserialize>::deserialize_in_place(&mut self.0, deserializer)
                    }
                }
            }
        }
    } else {
        // Empty struct.

        quote! {
            impl #generics ::snapshot::Deserialize for #ident <#(#generic_type_idents,)*> {
                #[inline]
                unsafe fn deserialize<R>(_: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<Self>
                where
                    R: ::snapshot::ReadUninit,
                {
                    Ok(Self)
                }
            }
        }
    }
}

fn derive_deserialize_enum(
    ident: &Ident,
    generics: &Generics,
    data_enum: &DataEnum,
) -> proc_macro2::TokenStream {
    let generic_type_idents = generics.type_params().map(|param| &param.ident);

    let variants = data_enum.variants.iter().enumerate().map(|(i, variant)| {
        let i = u8::try_from(i).expect("too many variants");
        let variant_ident = &variant.ident;

        // Check if the variant is a tuple field.
        if variant
            .fields
            .iter()
            .next()
            .is_some_and(|field| field.ident.is_none())
        {
            let field_types = variant.fields.iter().map(|field| &field.ty);

            return quote! {
                #i => {
                    Ok(Self::#variant_ident(#(unsafe {
                        <#field_types as ::snapshot::Deserialize>::deserialize(deserializer)?
                    },)*))
                }
            };
        }

        // Not a tuple field.
        let field_types = variant.fields.iter().map(|field| &field.ty);
        let field_idents = variant
            .fields
            .iter()
            .map(|field| field.ident.as_ref().unwrap());

        quote! {
            #i => {
                Ok(Self::#variant_ident { #(#field_idents: unsafe {
                    <#field_types as ::snapshot::Deserialize>::deserialize(deserializer)?
                }),* })
            }
        }
    });

    quote! {
        impl #generics ::snapshot::Deserialize for #ident <#(#generic_type_idents,)*> {
            #[inline]
            unsafe fn deserialize<R>(deserializer: &mut ::snapshot::Deserializer<R>) -> ::snapshot::Result<Self>
            where
                R: ::snapshot::ReadUninit,
            {
                match u8::deserialize(deserializer)? {
                    #(#variants,)*
                    _ => unreachable!(),
                }
            }
        }
    }
}
