//! # dm-macros —— 过程宏
//!
//! `#[derive(FromRow)]`（设计方案 §12 M5 / T5.3）：把一个结构体映射成一行结果。
//!
//! 设计约定：
//!
//! - 字段名默认按**大小写不敏感**匹配列名（与 `dm-query` 的 `ColumnIndex for &str` 一致）；
//! - 用 `#[dm(rename = "COL")]` 显式指定列名；
//! - `Option<T>` 字段接受 `NULL`，非 `Option` 字段遇到 `NULL` 报 `NullViolation`；
//! - 生成的代码只调用 `dm_query::Row` 的公开方法，不产生对 `dm-odbc` 的耦合。
//!
//! 刻意**不提供编译期 SQL 校验宏**（`dm_query!`）：它需要驱动提供 Describe 级别的
//! 可空性元数据，而达梦 ODBC 的 `describe_param` 实测不可用（对 `cast(? as varchar(10))`
//! 返回 `Varchar{length: 8188}`，§4.4）。该能力登记为 v1.2 备选。
//!
//! # 生成的代码长什么样
//!
//! ```ignore
//! impl ::dm_query::FromRow for User {
//!     fn from_row(row: &::dm_query::Row) -> ::std::result::Result<Self, ::dm_query::DmError> {
//!         ::std::result::Result::Ok(Self {
//!             id: row.get::<i64>("ID").map_err(|mut err| { /* 补一条「哪个字段」的诊断 */ err })?,
//!             name: row.get::<String>("name").map_err(/* … */)?,
//!         })
//!     }
//! }
//! ```
//!
//! 三处细节是有意的：
//!
//! 1. **路径全限定**（`::dm_query::…`）：不往调用方的命名空间里塞任何名字，
//!    调用方不必 `use` 我们的类型（`serde` 同款做法）。
//! 2. **每个字段都推一条诊断**：`Row::get` 报出来的错误带的是**列名**，而调用方
//!    看到的是**结构体字段名**——两者被 `rename` 拆开时（`#[dm(rename = "C_NAME")]`
//!    映射到 `name`）只有把两者都写进诊断，报错才定位得到。
//! 3. **每个字段类型都加 `Decode` 约束**：泛型结构体也能用，且约束错在字段类型上
//!    时编译器报的是「哪个类型没实现 `Decode`」，不是一句「where 子句不满足」。

#![forbid(unsafe_code)]

use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, parse_macro_input, parse_quote};

/// 把一个结构体映射成 `dm_query::FromRow`。
///
/// 只支持**具名字段**的结构体：元组结构体的字段没有名字，也就没有默认列名。
///
/// # 属性
///
/// - `#[dm(rename = "列名")]`：显式指定该字段读哪一列（不加时用字段名）。
///
/// # Panics
///
/// 不 panic：所有错误（非结构体、元组结构体、`dm` 属性写法不对）都以
/// `compile_error!` 的形式返回，这样 IDE 里能定位到具体的 span。
#[proc_macro_derive(FromRow, attributes(dm))]
pub fn derive_from_row(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand(&input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            Fields::Unnamed(_) => {
                return Err(syn::Error::new_spanned(
                    input,
                    "`#[derive(FromRow)]` 只支持具名字段的结构体：元组结构体的字段没有名字，\
                     也就没有默认列名。要给每个字段指定列名，请改成具名字段。",
                ));
            }
            Fields::Unit => {
                return Err(syn::Error::new_spanned(
                    input,
                    "`#[derive(FromRow)]` 不支持空结构体：它一列都不读，请直接用 `()`。",
                ));
            }
        },
        Data::Enum(_) | Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "`#[derive(FromRow)]` 只能用在结构体上。",
            ));
        }
    };

    let mut column_of = Vec::new();
    let mut field_names = Vec::new();
    let mut field_types = Vec::new();

    for field in fields {
        let ident = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new_spanned(field, "具名字段必然有名字"))?;
        let column = column_name(field, ident)?;
        field_names.push(ident);
        field_types.push(field.ty.clone());
        column_of.push(LitStr::new(&column, ident.span()));
    }

    // 每个字段类型都要 `Decode`（泛型结构体也就能用了）。
    let mut generics = input.generics.clone();
    {
        let where_clause = generics.make_where_clause();
        for ty in &field_types {
            where_clause
                .predicates
                .push(parse_quote!(#ty: ::dm_query::Decode));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let reads =
        field_names
            .iter()
            .zip(&column_of)
            .zip(&field_types)
            .map(|((ident, column), ty)| {
                quote! {
                    #ident: row.get::<#ty>(#column).map_err(|mut err| {
                        err.push_diag(::dm_query::DiagRecord::native(
                            0,
                            ::std::concat!(
                                "字段 `", ::std::stringify!(#ident), "` 映射自列 `", #column, "`"
                            ),
                        ));
                        err
                    })?,
                }
            });

    Ok(quote! {
        impl #impl_generics ::dm_query::FromRow for #name #ty_generics #where_clause {
            fn from_row(
                row: &::dm_query::Row,
            ) -> ::std::result::Result<Self, ::dm_query::DmError> {
                ::std::result::Result::Ok(Self {
                    #(#reads)*
                })
            }
        }
    })
}

/// 该字段读哪一列：`#[dm(rename = "…")]` 优先，否则用字段名。
fn column_name(field: &syn::Field, ident: &syn::Ident) -> syn::Result<String> {
    let mut renamed: Option<String> = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("dm") {
            continue;
        }
        let mut seen_key = false;
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("rename") {
                return Err(meta.error(
                    "只认 `#[dm(rename = \"列名\")]`：其它键（例如 `skip`/`default`）\
                     还没有语义，加进来会让「字段没读列」变成静默行为",
                ));
            }
            seen_key = true;
            let value: LitStr = meta.value()?.parse()?;
            renamed = Some(value.value());
            Ok(())
        })?;
        if !seen_key {
            return Err(syn::Error::new_spanned(
                attr,
                "`#[dm(...)]` 里没写东西：想改列名请写 `#[dm(rename = \"列名\")]`",
            ));
        }
    }
    let column = renamed.unwrap_or_else(|| ident.to_string());
    if column.is_empty() {
        return Err(syn::Error::new_spanned(
            ident,
            "列名不能是空串：`Row::get(\"\")` 永远找不到列。",
        ));
    }
    Ok(column)
}
