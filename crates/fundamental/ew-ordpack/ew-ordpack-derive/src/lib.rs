//! Derive macros for `ew-ordpack`.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use syn::{
    parse_macro_input, parse_quote, punctuated::Punctuated, Attribute, Data, DeriveInput, Fields,
    GenericParam, Generics, Ident, Index, LitStr, Path, Token, Type, Variant, WhereClause,
    WherePredicate,
};

// ---------------------------------------------------------------------------
// Attributes
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ContainerAttrs {
    krate: Option<Path>,
    transparent: bool,
    packed: bool,
    compat: bool,
    bound: Option<Punctuated<WherePredicate, Token![,]>>,
}

#[derive(Clone, Default)]
enum DefaultKind {
    #[default]
    Trait,
    Path(Path),
}

#[derive(Default)]
struct FieldAttrs {
    skip: bool,
    default: Option<DefaultKind>,
    with: Option<Path>,
    rename: Option<String>,
    sort: bool,
}

#[derive(Default)]
struct VariantAttrs {
    tag: Option<u64>,
    rename: Option<String>,
}

fn parse_container_attrs(attrs: &[Attribute]) -> syn::Result<ContainerAttrs> {
    let mut out = ContainerAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("ordpack") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("crate") {
                let value = meta.value()?;
                let s: LitStr = value.parse()?;
                out.krate = Some(s.parse()?);
                Ok(())
            } else if meta.path.is_ident("transparent") {
                out.transparent = true;
                Ok(())
            } else if meta.path.is_ident("packed") {
                out.packed = true;
                Ok(())
            } else if meta.path.is_ident("bound") {
                let value = meta.value()?;
                let s: LitStr = value.parse()?;
                let clause: WhereClause = syn::parse_str(&format!("where {}", s.value()))?;
                out.bound = Some(clause.predicates);
                Ok(())
            } else if meta.path.is_ident("rename") {
                let _: LitStr = meta.value()?.parse()?;
                Ok(())
            } else if meta.path.is_ident("compat") {
                out.compat = true;
                Ok(())
            } else {
                Err(meta.error("unknown ew-ordpack container attribute"))
            }
        })?;
    }
    Ok(out)
}

fn parse_field_attrs(attrs: &[Attribute]) -> syn::Result<FieldAttrs> {
    let mut out = FieldAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("ordpack") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                out.skip = true;
                Ok(())
            } else if meta.path.is_ident("default") {
                if meta.input.peek(Token![=]) {
                    let s: LitStr = meta.value()?.parse()?;
                    out.default = Some(DefaultKind::Path(s.parse()?));
                } else {
                    out.default = Some(DefaultKind::Trait);
                }
                Ok(())
            } else if meta.path.is_ident("with") {
                let s: LitStr = meta.value()?.parse()?;
                out.with = Some(s.parse()?);
                Ok(())
            } else if meta.path.is_ident("rename") {
                let s: LitStr = meta.value()?.parse()?;
                out.rename = Some(s.value());
                Ok(())
            } else if meta.path.is_ident("sort") {
                out.sort = true;
                Ok(())
            } else if meta.path.is_ident("transparent") || meta.path.is_ident("packed") {
                Err(meta.error("type-level attribute used on a field"))
            } else {
                Err(meta.error("unknown ew-ordpack field attribute"))
            }
        })?;
    }
    Ok(out)
}

fn parse_variant_attrs(attrs: &[Attribute]) -> syn::Result<VariantAttrs> {
    let mut out = VariantAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("ordpack") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("tag") {
                let value = meta.value()?;
                let lit: syn::LitInt = value.parse()?;
                out.tag = Some(lit.base10_parse()?);
                Ok(())
            } else if meta.path.is_ident("rename") {
                let s: LitStr = meta.value()?.parse()?;
                out.rename = Some(s.value());
                Ok(())
            } else {
                Err(meta.error("unknown ew-ordpack variant attribute"))
            }
        })?;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Field model
// ---------------------------------------------------------------------------

struct FieldInfo {
    ident: Option<Ident>,
    index: usize,
    ty: Type,
    name: String,
    skip: bool,
    default: Option<DefaultKind>,
    with: Option<Path>,
    sort: bool,
    wire_index: u16,
}

impl FieldInfo {
    fn bound_access(&self) -> TokenStream2 {
        match &self.ident {
            Some(ident) => quote!(&self.#ident),
            None => {
                let index = Index::from(self.index);
                quote!(&self.#index)
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldsKind {
    Named,
    Unnamed,
    Unit,
}

fn collect_fields(fields: &Fields) -> syn::Result<(Vec<FieldInfo>, FieldsKind)> {
    let kind = match fields {
        Fields::Named(_) => FieldsKind::Named,
        Fields::Unnamed(_) => FieldsKind::Unnamed,
        Fields::Unit => FieldsKind::Unit,
    };
    let mut out = Vec::new();
    let mut wire_index = 0u16;
    for (index, field) in fields.iter().enumerate() {
        let attrs = parse_field_attrs(&field.attrs)?;
        let name = match (&field.ident, attrs.rename.clone()) {
            (_, Some(rename)) => rename,
            (Some(ident), None) => ident.to_string(),
            (None, None) => index.to_string(),
        };
        out.push(FieldInfo {
            ident: field.ident.clone(),
            index,
            ty: field.ty.clone(),
            name,
            skip: attrs.skip,
            default: attrs.default,
            with: attrs.with,
            sort: attrs.sort,
            wire_index,
        });
        if !attrs.skip {
            wire_index += 1;
        }
    }
    Ok((out, kind))
}

fn encoded_count(fields: &[FieldInfo]) -> u16 {
    fields.iter().filter(|field| !field.skip).count() as u16
}

// ---------------------------------------------------------------------------
// Generics
// ---------------------------------------------------------------------------

fn encode_generics(
    generics: &Generics,
    krate: &Path,
    extra: Option<Punctuated<WherePredicate, Token![,]>>,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    let mut generics = generics.clone();
    for param in generics.params.iter_mut() {
        if let GenericParam::Type(param) = param {
            param.bounds.push(parse_quote!(#krate::Encode));
        }
    }
    if let Some(extra) = extra {
        generics.make_where_clause().predicates.extend(extra);
    }
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    (
        impl_generics.to_token_stream(),
        where_clause.to_token_stream(),
        TokenStream2::new(),
    )
}

fn decode_generics(
    generics: &Generics,
    krate: &Path,
    extra: Option<Punctuated<WherePredicate, Token![,]>>,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    let mut generics = generics.clone();
    generics.params.insert(0, parse_quote!('__de));
    let mut extra_predicates: Vec<WherePredicate> = Vec::new();
    for param in generics.params.iter_mut() {
        match param {
            GenericParam::Type(param) => {
                param.bounds.push(parse_quote!(#krate::Decode<'__de>));
            }
            GenericParam::Lifetime(param) => {
                let lifetime = param.lifetime.clone();
                extra_predicates.push(parse_quote!('__de: #lifetime));
            }
            GenericParam::Const(_) => {}
        }
    }
    if !extra_predicates.is_empty() {
        generics
            .make_where_clause()
            .predicates
            .extend(extra_predicates);
    }
    if let Some(extra) = extra {
        generics.make_where_clause().predicates.extend(extra);
    }
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    (
        impl_generics.to_token_stream(),
        where_clause.to_token_stream(),
        TokenStream2::new(),
    )
}

fn type_generics(generics: &Generics) -> TokenStream2 {
    let (_, ty_generics, _) = generics.split_for_impl();
    ty_generics.to_token_stream()
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

struct EncodeParts {
    fingerprint: TokenStream2,
    kind: TokenStream2,
    encoded_len: TokenStream2,
    encode: TokenStream2,
}

fn struct_encode_parts(
    fields: &[FieldInfo],
    kind: FieldsKind,
    attrs: &ContainerAttrs,
    krate: &Path,
    name: &str,
) -> syn::Result<EncodeParts> {
    let count = encoded_count(fields);
    let encoded: Vec<&FieldInfo> = fields.iter().filter(|field| !field.skip).collect();

    let fingerprint_fields = encoded.iter().map(|field| {
        let ty = &field.ty;
        quote!(<#ty as #krate::Encode>::KIND)
    });

    let is_transparent =
        attrs.transparent || (kind == FieldsKind::Unnamed && count == 1 && fields.len() == 1);
    if attrs.transparent && count != 1 {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "`transparent` requires exactly one encoded field",
        ));
    }

    if attrs.compat {
        if attrs.packed {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`compat` and `packed` are mutually exclusive",
            ));
        }
        if is_transparent {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`compat` cannot be used on a transparent type",
            ));
        }
        if encoded.iter().any(|field| field.with.is_some()) {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`compat` requires exact field lengths; remove `with` fields",
            ));
        }
    }

    let fingerprint = quote!(#krate::fingerprint(
        concat!(module_path!(), "::", #name),
        &[#count as u64, #(#fingerprint_fields),*],
    ));
    let kind = quote!(#krate::fingerprint_init(concat!(module_path!(), "::", #name)));

    if is_transparent {
        let field = encoded[0];
        let access = field.bound_access();
        let ty = &field.ty;
        return Ok(EncodeParts {
            fingerprint,
            kind: quote!(<#ty as #krate::Encode>::KIND),
            encoded_len: quote!(#krate::Encode::encoded_len(#access)),
            encode: quote!(#krate::Encode::encode(#access, __e)),
        });
    }

    let base = if attrs.packed { 0usize } else { 2usize };

    let writes = encoded.iter().map(|field| {
        let access = field.bound_access();
        if field.sort {
            quote!(#krate::SortedEncode::encode_sorted(#access, __e)?;)
        } else {
            match &field.with {
                Some(with) => quote!(#with::encode(#access, __e)?;),
                None => quote!(#krate::Encode::encode(#access, __e)?;),
            }
        }
    });

    let has_custom = encoded.iter().any(|field| field.with.is_some());
    let plain_encoded_len = if has_custom {
        quote!(::core::option::Option::None)
    } else {
        let lens = encoded.iter().map(|field| {
            let access = field.bound_access();
            quote!(__len = __len.checked_add(#krate::Encode::encoded_len(#access)?)?;)
        });
        if attrs.packed {
            quote!({
                let mut __len = 0usize;
                #(#lens)*
                ::core::option::Option::Some(__len)
            })
        } else {
            quote!({
                let mut __len = #base;
                #(#lens)*
                ::core::option::Option::Some(__len)
            })
        }
    };

    if attrs.compat {
        let payload_lens: Vec<TokenStream2> = encoded
            .iter()
            .map(|field| {
                let access = field.bound_access();
                quote!(__p = #krate::optional_add(__p, #krate::Encode::encoded_len(#access));)
            })
            .collect();

        let encoded_len = quote!({
            let mut __p = ::core::option::Option::Some(0usize);
            #(#payload_lens)*
            match __p {
                ::core::option::Option::Some(__p) => ::core::option::Option::Some(
                    2 + #krate::varint_len(__p as u64) + __p,
                ),
                ::core::option::Option::None => ::core::option::Option::None,
            }
        });

        let encode = quote!({
            let mut __p = ::core::option::Option::Some(0usize);
            #(#payload_lens)*
            let __p = match __p {
                ::core::option::Option::Some(__p) => __p,
                ::core::option::Option::None => {
                    return ::core::result::Result::Err(#krate::Error::new(
                        #krate::ErrorKind::Unsupported(
                            "compat requires exact field lengths",
                        ),
                    ));
                }
            };
            __e.write_field_count(#count)?;
            __e.write_varint(__p as u64)?;
            #(#writes)*
            ::core::result::Result::Ok(())
        });

        return Ok(EncodeParts {
            fingerprint,
            kind,
            encoded_len,
            encode,
        });
    }

    let count_write = if attrs.packed {
        quote!()
    } else {
        quote!(__e.write_field_count(#count)?;)
    };

    Ok(EncodeParts {
        fingerprint,
        kind,
        encoded_len: plain_encoded_len,
        encode: quote!({
            #count_write
            #(#writes)*
            ::core::result::Result::Ok(())
        }),
    })
}

fn enum_encode_parts(
    variants: &Punctuated<Variant, Token![,]>,
    attrs: &ContainerAttrs,
    krate: &Path,
    name: &str,
) -> syn::Result<EncodeParts> {
    let mut fingerprint_items = Vec::new();
    let mut encode_arms = Vec::new();
    let mut len_arms = Vec::new();
    let variant_count = variants.len();
    fingerprint_items.push(quote!(#variant_count as u64));

    for (index, variant) in variants.iter().enumerate() {
        let vattrs = parse_variant_attrs(&variant.attrs)?;
        let tag = vattrs.tag.unwrap_or(index as u64);
        let (fields, kind) = collect_fields(&variant.fields)?;
        let count = encoded_count(&fields);
        let vident = &variant.ident;

        fingerprint_items.push(quote!(#tag));
        for field in fields.iter().filter(|field| !field.skip) {
            let ty = &field.ty;
            fingerprint_items.push(quote!(<#ty as #krate::Encode>::KIND));
        }

        if count == 0 {
            encode_arms.push(quote!(Self::#vident => {
                __e.write_tag(#tag)?;
            }));
            len_arms.push(quote!(Self::#vident => ::core::option::Option::Some(
                #krate::varint_len(#tag)
            )));
            continue;
        }

        let bindings: Vec<TokenStream2> = fields
            .iter()
            .enumerate()
            .map(|(i, field)| match &field.ident {
                Some(ident) => quote!(#ident),
                None => {
                    let ident = Ident::new(&format!("__f{}", i), proc_macro2::Span::call_site());
                    quote!(#ident)
                }
            })
            .collect();

        let pattern = if kind == FieldsKind::Named {
            quote!(Self::#vident { #(#bindings),* })
        } else {
            quote!(Self::#vident(#(#bindings),*))
        };

        let writes = fields
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.skip)
            .map(|(i, field)| {
                let binding = &bindings[i];
                if field.sort {
                    quote!(#krate::SortedEncode::encode_sorted(#binding, __e)?;)
                } else {
                    match &field.with {
                        Some(with) => quote!(#with::encode(#binding, __e)?;),
                        None => quote!(#krate::Encode::encode(#binding, __e)?;),
                    }
                }
            });

        let count_write = if attrs.packed {
            quote!()
        } else {
            quote!(__e.write_field_count(#count)?;)
        };

        encode_arms.push(quote!(#pattern => {
            __e.write_tag(#tag)?;
            #count_write
            #(#writes)*
        }));

        let has_custom = fields.iter().any(|field| field.with.is_some());
        if has_custom {
            len_arms.push(quote!(#pattern => ::core::option::Option::None));
        } else {
            let lens = fields
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.skip)
                .map(|(i, _)| {
                    let binding = &bindings[i];
                    quote!(__len = __len.checked_add(#krate::Encode::encoded_len(#binding)?)?;)
                });
            let base = if attrs.packed { 0usize } else { 2usize };
            len_arms.push(quote!(#pattern => {
                let mut __len = #krate::varint_len(#tag) + #base;
                #(#lens)*
                ::core::option::Option::Some(__len)
            }));
        }
    }

    Ok(EncodeParts {
        fingerprint: quote!(#krate::fingerprint(
            concat!(module_path!(), "::", #name),
            &[#(#fingerprint_items),*],
        )),
        kind: quote!(#krate::fingerprint_init(concat!(module_path!(), "::", #name))),
        encoded_len: quote!(match self {
            #(#len_arms),*
        }),
        encode: quote!({
            match self {
                #(#encode_arms),*
            }
            ::core::result::Result::Ok(())
        }),
    })
}

fn expand_encode(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let attrs = parse_container_attrs(&input.attrs)?;
    let krate = attrs
        .krate
        .clone()
        .unwrap_or_else(|| parse_quote!(::ew_ordpack));
    let name = &input.ident;
    let name_str = name.to_string();
    let (impl_generics, where_clause, _) =
        encode_generics(&input.generics, &krate, attrs.bound.clone());
    let ty_generics = type_generics(&input.generics);

    let parts = match &input.data {
        Data::Struct(data) => {
            let (fields, kind) = collect_fields(&data.fields)?;
            struct_encode_parts(&fields, kind, &attrs, &krate, &name_str)?
        }
        Data::Enum(data) => {
            if attrs.compat {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "`compat` is only supported on structs",
                ));
            }
            enum_encode_parts(&data.variants, &attrs, &krate, &name_str)?
        }
        Data::Union(_) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "unions are not supported",
            ))
        }
    };

    let EncodeParts {
        fingerprint,
        kind,
        encoded_len,
        encode,
    } = parts;

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::Encode for #name #ty_generics #where_clause {
            const FINGERPRINT: u64 = #fingerprint;
            const KIND: u64 = #kind;

            #[inline]
            fn encoded_len(&self) -> ::core::option::Option<usize> {
                #encoded_len
            }

            #[inline(always)]
            #[allow(unused_variables)]
            fn encode<__W: #krate::Writer>(
                &self,
                __e: &mut #krate::Encoder<__W>,
            ) -> ::core::result::Result<(), #krate::Error> {
                #encode
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

fn struct_decode_parts(
    fields: &[FieldInfo],
    kind: FieldsKind,
    attrs: &ContainerAttrs,
    krate: &Path,
) -> syn::Result<TokenStream2> {
    let count = encoded_count(fields);
    let encoded: Vec<&FieldInfo> = fields.iter().filter(|field| !field.skip).collect();
    let is_transparent =
        attrs.transparent || (kind == FieldsKind::Unnamed && count == 1 && fields.len() == 1);

    if is_transparent {
        if count != 1 {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`transparent` requires exactly one encoded field",
            ));
        }
        let field = encoded[0];
        let ty = &field.ty;
        let ctor = ctor_for(fields, kind);
        let binding = binding_for(field);
        return Ok(quote!({
            let #binding = <#ty as #krate::Decode<'__de>>::decode(__d)?;
            ::core::result::Result::Ok(#ctor)
        }));
    }

    let ctor = ctor_for(fields, kind);

    // Fast path: the field count matches exactly, so every field can be decoded
    // straight through without a per-field presence check. Skipped fields are
    // still initialised so the struct literal is complete.
    let mut fast = Vec::new();
    for field in fields.iter().filter(|field| field.skip) {
        let binding = binding_for(field);
        let default = default_expr(field, krate)?;
        fast.push(quote!(let #binding = #default;));
    }
    for field in &encoded {
        let binding = binding_for(field);
        let ty = &field.ty;
        let expr = match &field.with {
            Some(with) => quote!(#with::decode(__d)?),
            None => quote!(<#ty as #krate::Decode<'__de>>::decode(__d)?),
        };
        fast.push(quote!(let #binding = #expr;));
    }

    if attrs.packed {
        return Ok(quote!({
            #(#fast)*
            ::core::result::Result::Ok(#ctor)
        }));
    }

    let mut slow = Vec::new();
    slow.push(quote! {
        if __n > #count {
            return ::core::result::Result::Err(__d.err(
                #krate::ErrorKind::FieldCount { got: __n, expected: #count }
            ));
        }
    });

    for field in fields {
        let binding = binding_for(field);
        let ty = &field.ty;
        if field.skip {
            let default = default_expr(field, krate)?;
            slow.push(quote! {
                let #binding = #default;
            });
            continue;
        }
        let wire_index = field.wire_index;
        let name = &field.name;
        let fallback = match &field.default {
            Some(_) => default_expr(field, krate)?,
            None => quote!(return ::core::result::Result::Err(__d.missing(#name))),
        };
        match &field.with {
            Some(with) => {
                slow.push(quote! {
                    let #binding = if __n > #wire_index {
                        #with::decode(__d)?
                    } else {
                        #fallback
                    };
                });
            }
            None => {
                slow.push(quote! {
                    let #binding = if __n > #wire_index {
                        <#ty as #krate::Decode<'__de>>::decode(__d)?
                    } else {
                        #fallback
                    };
                });
            }
        }
    }

    if attrs.compat {
        return Ok(quote!({
            let __n = __d.read_field_count()?;
            let __len = __d.read_len()?;
            if __len > __d.remaining() {
                return ::core::result::Result::Err(__d.err(#krate::ErrorKind::UnexpectedEof));
            }
            let __end = match __d.position().checked_add(__len) {
                ::core::option::Option::Some(end) => end,
                ::core::option::Option::None => {
                    return ::core::result::Result::Err(
                        __d.err(#krate::ErrorKind::LengthOverflow),
                    );
                }
            };
            let __value = if __n == #count {
                #(#fast)*
                #ctor
            } else if __n < #count {
                #(#slow)*
                #ctor
            } else {
                #(#fast)*
                let __value = #ctor;
                let __pos = __d.position();
                if __pos > __end {
                    return ::core::result::Result::Err(
                        __d.err(#krate::ErrorKind::Custom("compat payload overflow")),
                    );
                }
                __d.skip(__end - __pos)?;
                __value
            };
            if __d.position() != __end {
                return ::core::result::Result::Err(
                    __d.err(#krate::ErrorKind::Custom("compat length mismatch")),
                );
            }
            ::core::result::Result::Ok(__value)
        }));
    }

    Ok(quote!({
        let __n = __d.read_field_count()?;
        if __n == #count {
            #(#fast)*
            ::core::result::Result::Ok(#ctor)
        } else {
            #(#slow)*
            ::core::result::Result::Ok(#ctor)
        }
    }))
}

fn enum_decode_parts(
    variants: &Punctuated<Variant, Token![,]>,
    attrs: &ContainerAttrs,
    krate: &Path,
) -> syn::Result<TokenStream2> {
    let mut arms = Vec::new();
    let variant_count = variants.len();

    for (index, variant) in variants.iter().enumerate() {
        let vattrs = parse_variant_attrs(&variant.attrs)?;
        let tag = vattrs.tag.unwrap_or(index as u64);
        let (fields, kind) = collect_fields(&variant.fields)?;
        let count = encoded_count(&fields);
        let vident = &variant.ident;

        if count == 0 {
            arms.push(quote!(#tag => ::core::result::Result::Ok(Self::#vident)));
            continue;
        }

        let mut statements = Vec::new();
        if !attrs.packed {
            statements.push(quote! {
                let __n = __d.read_field_count()?;
                if __n > #count {
                    return ::core::result::Result::Err(__d.err(
                        #krate::ErrorKind::FieldCount { got: __n, expected: #count }
                    ));
                }
            });
        }

        for field in &fields {
            let binding = binding_for(field);
            let ty = &field.ty;
            if field.skip {
                let default = default_expr(field, krate)?;
                statements.push(quote! { let #binding = #default; });
                continue;
            }
            let wire_index = field.wire_index;
            let name = &field.name;
            let fallback = match &field.default {
                Some(_) => default_expr(field, krate)?,
                None => quote!(return ::core::result::Result::Err(__d.missing(#name))),
            };
            let expr = if attrs.packed {
                quote!(<#ty as #krate::Decode<'__de>>::decode(__d)?)
            } else {
                quote! {
                    if __n > #wire_index {
                        <#ty as #krate::Decode<'__de>>::decode(__d)?
                    } else {
                        #fallback
                    }
                }
            };
            statements.push(quote! { let #binding = #expr; });
        }

        let bindings: Vec<TokenStream2> = fields.iter().map(binding_for).collect();
        let ctor = match kind {
            FieldsKind::Named => quote!(Self::#vident { #(#bindings),* }),
            FieldsKind::Unnamed => quote!(Self::#vident(#(#bindings),*)),
            FieldsKind::Unit => quote!(Self::#vident),
        };
        arms.push(quote!(#tag => {
            #(#statements)*
            ::core::result::Result::Ok(#ctor)
        }));
    }

    let tag_lit = variant_count;
    Ok(quote!({
        let __tag = __d.read_tag()?;
        match __tag {
            #(#arms,)*
            _ => ::core::result::Result::Err(__d.err(
                #krate::ErrorKind::InvalidVariant {
                    got: __tag,
                    variants: #tag_lit as u64,
                }
            )),
        }
    }))
}

fn binding_for(field: &FieldInfo) -> TokenStream2 {
    match &field.ident {
        Some(ident) => quote!(#ident),
        None => {
            let ident = Ident::new(
                &format!("__f{}", field.index),
                proc_macro2::Span::call_site(),
            );
            quote!(#ident)
        }
    }
}

fn ctor_for(fields: &[FieldInfo], kind: FieldsKind) -> TokenStream2 {
    let bindings: Vec<TokenStream2> = fields.iter().map(binding_for).collect();
    match kind {
        FieldsKind::Named => quote!(Self { #(#bindings),* }),
        FieldsKind::Unnamed => quote!(Self(#(#bindings),*)),
        FieldsKind::Unit => quote!(Self),
    }
}

fn default_expr(field: &FieldInfo, _krate: &Path) -> syn::Result<TokenStream2> {
    let ty = &field.ty;
    Ok(match &field.default {
        Some(DefaultKind::Path(path)) => quote!(#path()),
        _ => quote!(<#ty as ::core::default::Default>::default()),
    })
}

fn expand_decode(input: &DeriveInput) -> syn::Result<TokenStream2> {
    let attrs = parse_container_attrs(&input.attrs)?;
    let krate = attrs
        .krate
        .clone()
        .unwrap_or_else(|| parse_quote!(::ew_ordpack));
    let name = &input.ident;
    let (impl_generics, where_clause, _) =
        decode_generics(&input.generics, &krate, attrs.bound.clone());
    let ty_generics = type_generics(&input.generics);

    let body = match &input.data {
        Data::Struct(data) => {
            let (fields, kind) = collect_fields(&data.fields)?;
            struct_decode_parts(&fields, kind, &attrs, &krate)?
        }
        Data::Enum(data) => {
            if attrs.compat {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    "`compat` is only supported on structs",
                ));
            }
            enum_decode_parts(&data.variants, &attrs, &krate)?
        }
        Data::Union(_) => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "unions are not supported",
            ))
        }
    };

    Ok(quote! {
        #[automatically_derived]
        impl #impl_generics #krate::Decode<'__de> for #name #ty_generics #where_clause {
            #[inline(always)]
            #[allow(unused_variables)]
            fn decode<__R: #krate::Reader<'__de>>(
                __d: &mut #krate::Decoder<'__de, __R>,
            ) -> ::core::result::Result<Self, #krate::Error> {
                #body
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// Derive [`Encode`](https://docs.rs/ew-ordpack/latest/ew_ordpack/trait.Encode.html).
#[proc_macro_derive(Encode, attributes(ordpack))]
pub fn derive_encode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_encode(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive [`Decode`](https://docs.rs/ew-ordpack/latest/ew_ordpack/trait.Decode.html).
#[proc_macro_derive(Decode, attributes(ordpack))]
pub fn derive_decode(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_decode(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
