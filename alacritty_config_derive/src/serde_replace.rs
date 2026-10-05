use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{
    Data, DataStruct, DeriveInput, Error, Field, Fields, Generics, Ident, parse_macro_input,
};

use crate::{Attr, GenericsStreams, MULTIPLE_FLATTEN_ERROR};

/// Error if the derive was used on an unsupported type.
const UNSUPPORTED_ERROR: &str = "SerdeReplace must be used on a tuple struct";

pub fn derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match input.data {
        Data::Struct(DataStruct { fields: Fields::Unnamed(_), .. }) | Data::Enum(_) => {
            derive_direct(input.ident, input.generics).into()
        },
        Data::Struct(DataStruct { fields: Fields::Named(fields), .. }) => {
            derive_recursive(input.ident, input.generics, fields.named).into()
        },
        _ => Error::new(input.ident.span(), UNSUPPORTED_ERROR).to_compile_error().into(),
    }
}

pub fn derive_direct(ident: Ident, generics: Generics) -> TokenStream2 {
    quote! {
        impl <#generics> alacritty_config::SerdeReplace for #ident <#generics> {
            fn replace(&mut self, value: toml::Value) -> Result<(), Box<dyn std::error::Error>> {
                *self = serde::Deserialize::deserialize(value)?;

                Ok(())
            }
        }
    }
}

pub fn derive_recursive<T>(
    ident: Ident,
    generics: Generics,
    fields: Punctuated<Field, T>,
) -> TokenStream2 {
    let GenericsStreams { unconstrained, constrained, .. } =
        crate::generics_streams(&generics.params);
    let (replace_arms, flatten) = match match_arms(&fields) {
        Err(e) => return e.to_compile_error(),
        Ok(replace_arms) => replace_arms,
    };

    // With a flattened field, all keys unknown to this struct are forwarded to
    // it in a single batch, mirroring how the deserializer drains its unused
    // keys into the flattened struct.
    let table_stream = if let Some(flatten_ident) = flatten {
        quote! {
            let mut unmatched = toml::Table::new();

            for (field, next_value) in table {
                match field.as_str() {
                    #replace_arms
                    _ => {
                        unmatched.insert(field, next_value);
                    },
                }
            }

            if !unmatched.is_empty() {
                alacritty_config::SerdeReplace::replace(
                    &mut self.#flatten_ident,
                    toml::Value::Table(unmatched),
                )?;
            }
        }
    } else {
        quote! {
            for (field, next_value) in table {
                match field.as_str() {
                    #replace_arms
                    _ => {
                        let error = format!("Field \"{}\" does not exist", field);
                        return Err(error.into());
                    },
                }
            }
        }
    };

    quote! {
        #[allow(clippy::extra_unused_lifetimes)]
        impl <'de, #constrained> alacritty_config::SerdeReplace for #ident <#unconstrained> {
            fn replace(&mut self, value: toml::Value) -> Result<(), Box<dyn std::error::Error>> {
                match value {
                    toml::Value::Table(table) => {
                        #table_stream
                    },
                    value => *self = serde::Deserialize::deserialize(value)?,
                }

                Ok(())
            }
        }
    }
}

/// Create SerdeReplace recursive match arms, returning the arms along with
/// the flattened field which unmatched keys are forwarded to, if any.
fn match_arms<T>(
    fields: &Punctuated<Field, T>,
) -> Result<(TokenStream2, Option<Ident>), syn::Error> {
    let mut stream = TokenStream2::default();
    let mut flattened_field = None;

    // Create arm for each field.
    for field in fields {
        let ident = field.ident.as_ref().expect("unreachable tuple struct");
        let literal = ident.to_string();

        // Check if #[config(flattened)] attribute is present.
        let flatten = field
            .attrs
            .iter()
            .filter(|attr| (*attr).path().is_ident("config"))
            .filter_map(|attr| attr.parse_args::<Attr>().ok())
            .any(|parsed| parsed.ident.as_str() == "flatten");

        if flatten && flattened_field.is_some() {
            return Err(Error::new(ident.span(), MULTIPLE_FLATTEN_ERROR));
        } else if flatten {
            flattened_field = Some(ident.clone());
            continue;
        }

        // Skip fields which are not part of the user configuration, just like
        // the deserializer does.
        let skip = field
            .attrs
            .iter()
            .filter(|attr| (*attr).path().is_ident("config"))
            .filter_map(|attr| attr.parse_args::<Attr>().ok())
            .any(|parsed| parsed.ident.as_str() == "skip");

        if skip {
            continue;
        }

        // Extract all `#[config(alias = "...")]` attribute values.
        let aliases = field
            .attrs
            .iter()
            .filter(|attr| (*attr).path().is_ident("config"))
            .filter_map(|attr| attr.parse_args::<Attr>().ok())
            .filter(|parsed| parsed.ident.as_str() == "alias")
            .map(|parsed| {
                let value = parsed
                    .param
                    .ok_or_else(|| format!("Field \"{ident}\" has no alias value"))?
                    .value();

                if value.trim().is_empty() {
                    return Err(format!("Field \"{ident}\" has an empty alias value"));
                }

                Ok(value)
            })
            .collect::<Result<Vec<String>, String>>()
            .map_err(|msg| Error::new(ident.span(), msg))?;

        stream.extend(quote! {
            #(#aliases)|* | #literal => {
                alacritty_config::SerdeReplace::replace(&mut self.#ident, next_value)?
            },
        });
    }

    Ok((stream, flattened_field))
}
