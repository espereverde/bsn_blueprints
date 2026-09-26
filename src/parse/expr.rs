//! Helpers for `ron2` expressions, and a serde deserializer over them for leaf values.
//!
//! Structured values (structs, enums, lists, ..) are read with the type registry in
//! [`super::value`]; values whose type only knows serde (numbers, strings, `Name`, ..) are
//! read through [`ExprDeserializer`].

use std::fmt;

use ron2::Value;
use ron2::ast::{Expr, StructBody, expr_to_value};
use ron2::value::Number;
use serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};

use crate::error::{BlueprintError, ErrorKind};

pub(crate) type ParseResult<T> = Result<T, BlueprintError>;

/// An error at `expr`.
pub(crate) fn error(kind: ErrorKind, expr: &Expr, message: impl Into<String>) -> BlueprintError {
    BlueprintError::new(kind, message).at(expr.span())
}

/// `Name(a, b, ..)`: the name and the positional arguments, if `expr` has that form.
pub(crate) fn call<'e, 'a>(expr: &'e Expr<'a>) -> Option<(&'e str, Vec<&'e Expr<'a>>)> {
    match expr {
        Expr::Struct(s) => match &s.body {
            Some(StructBody::Tuple(body)) => {
                Some((&s.name.name, body.elements.iter().map(|e| &e.expr).collect()))
            }
            None => Some((&s.name.name, Vec::new())),
            Some(StructBody::Fields(_)) => None,
        },
        _ => None,
    }
}

pub(crate) fn string<'e>(expr: &'e Expr) -> ParseResult<&'e str> {
    match expr {
        Expr::String(s) => Ok(&s.value),
        _ => Err(error(ErrorKind::Syntax, expr, "expected a string")),
    }
}

pub(crate) fn number(expr: &Expr) -> ParseResult<f64> {
    match expr_to_value(expr) {
        Ok(Value::Number(n)) => Ok(n.into_f64()),
        _ => Err(error(ErrorKind::Syntax, expr, "expected a number")),
    }
}

/// Deserializes a serde type from an expression.
pub(crate) struct ExprDeserializer<'e, 'a>(pub(crate) &'e Expr<'a>);

#[derive(Debug)]
pub(crate) struct ExprError(String);

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExprError {}

impl de::Error for ExprError {
    fn custom<T: fmt::Display>(message: T) -> Self {
        ExprError(message.to_string())
    }
}

impl<'de> de::Deserializer<'de> for ExprDeserializer<'_, '_> {
    type Error = ExprError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ExprError> {
        match self.0 {
            Expr::Unit(_) => visitor.visit_unit(),
            Expr::Bool(b) => visitor.visit_bool(b.value),
            Expr::Char(c) => visitor.visit_char(c.value),
            Expr::Byte(b) => visitor.visit_u8(b.value),
            Expr::String(s) => visitor.visit_str(&s.value),
            Expr::Bytes(b) => visitor.visit_bytes(&b.value),
            Expr::Number(_) => match expr_to_value(self.0).map_err(de::Error::custom)? {
                Value::Number(n) => visit_number(n, visitor),
                _ => Err(de::Error::custom("expected a number")),
            },
            Expr::Option(o) => match &o.value {
                Some(inner) => visitor.visit_some(ExprDeserializer(&inner.expr)),
                None => visitor.visit_none(),
            },
            Expr::Seq(s) => visitor.visit_seq(Items(s.items.iter().map(|i| &i.expr))),
            Expr::Tuple(t) => visitor.visit_seq(Items(t.elements.iter().map(|e| &e.expr))),
            Expr::Map(m) => visitor.visit_map(Entries {
                entries: m.entries.iter().map(|e| (Key::Expr(&e.key), &e.value)),
                value: None,
            }),
            Expr::AnonStruct(s) => visitor.visit_map(Entries {
                entries: s.fields.iter().map(|f| (Key::Name(&f.name.name), &f.value)),
                value: None,
            }),
            Expr::Struct(s) => match &s.body {
                None => visitor.visit_str(&s.name.name),
                Some(StructBody::Tuple(t)) => visitor.visit_seq(Items(t.elements.iter().map(|e| &e.expr))),
                Some(StructBody::Fields(f)) => visitor.visit_map(Entries {
                    entries: f.fields.iter().map(|f| (Key::Name(&f.name.name), &f.value)),
                    value: None,
                }),
            },
            Expr::Error(e) => Err(de::Error::custom(&e.error)),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, ExprError> {
        match self.0 {
            Expr::Option(_) => self.deserialize_any(visitor),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, ExprError> {
        match call(self.0) {
            Some((_, args)) if args.len() == 1 => visitor.visit_newtype_struct(ExprDeserializer(args[0])),
            _ => visitor.visit_newtype_struct(self),
        }
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ExprError> {
        match self.0 {
            Expr::Struct(_) | Expr::String(_) => visitor.visit_enum(Variant(self.0)),
            _ => Err(de::Error::custom("expected an enum variant")),
        }
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

fn visit_number<'de, V: Visitor<'de>>(n: Number, visitor: V) -> Result<V::Value, ExprError> {
    match n {
        Number::I8(v) => visitor.visit_i64(v.into()),
        Number::I16(v) => visitor.visit_i64(v.into()),
        Number::I32(v) => visitor.visit_i64(v.into()),
        Number::I64(v) => visitor.visit_i64(v),
        Number::U8(v) => visitor.visit_u64(v.into()),
        Number::U16(v) => visitor.visit_u64(v.into()),
        Number::U32(v) => visitor.visit_u64(v.into()),
        Number::U64(v) => visitor.visit_u64(v),
        other => visitor.visit_f64(other.into_f64()),
    }
}

struct Items<I>(I);

impl<'de, 'e, 'a: 'e, I: Iterator<Item = &'e Expr<'a>>> de::SeqAccess<'de> for Items<I> {
    type Error = ExprError;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, ExprError> {
        self.0.next().map(|expr| seed.deserialize(ExprDeserializer(expr))).transpose()
    }
}

enum Key<'e, 'a> {
    Expr(&'e Expr<'a>),
    Name(&'e str),
}

struct Entries<'e, 'a, I> {
    entries: I,
    value: Option<&'e Expr<'a>>,
}

impl<'de, 'e, 'a: 'e, I: Iterator<Item = (Key<'e, 'a>, &'e Expr<'a>)>> de::MapAccess<'de> for Entries<'e, 'a, I> {
    type Error = ExprError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, ExprError> {
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        match key {
            Key::Expr(expr) => seed.deserialize(ExprDeserializer(expr)).map(Some),
            Key::Name(name) => seed.deserialize(name.into_deserializer()).map(Some),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, ExprError> {
        let value = self.value.take().ok_or_else(|| de::Error::custom("value without key"))?;
        seed.deserialize(ExprDeserializer(value))
    }
}

/// An enum variant: `Name`, `Name(..)`, `Name(field: ..)`, or `"Name"`.
struct Variant<'e, 'a>(&'e Expr<'a>);

impl<'de> de::EnumAccess<'de> for Variant<'_, '_> {
    type Error = ExprError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), ExprError> {
        let name = match self.0 {
            Expr::Struct(s) => &s.name.name,
            Expr::String(s) => s.value.as_str(),
            _ => return Err(de::Error::custom("expected an enum variant")),
        };
        let variant = seed.deserialize(name.into_deserializer())?;
        Ok((variant, self))
    }
}

impl<'de> de::VariantAccess<'de> for Variant<'_, '_> {
    type Error = ExprError;

    fn unit_variant(self) -> Result<(), ExprError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, ExprError> {
        match call(self.0) {
            Some((_, args)) if args.len() == 1 => seed.deserialize(ExprDeserializer(args[0])),
            _ => Err(de::Error::custom("expected a variant with one value")),
        }
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value, ExprError> {
        match call(self.0) {
            Some((_, args)) => visitor.visit_seq(Items(args.into_iter())),
            None => Err(de::Error::custom("expected a tuple variant")),
        }
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, ExprError> {
        match self.0 {
            Expr::Struct(s) => match &s.body {
                Some(StructBody::Fields(f)) => visitor.visit_map(Entries {
                    entries: f.fields.iter().map(|f| (Key::Name(&f.name.name), &f.value)),
                    value: None,
                }),
                _ => Err(de::Error::custom("expected a struct variant")),
            },
            _ => Err(de::Error::custom("expected a struct variant")),
        }
    }
}
