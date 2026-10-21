//! Parse into the JSON data model without losing duplicate keys or YAML types.

use crate::{Error, MAX_DEPTH, MAX_DOCUMENT_BYTES};
use serde::de::{
    self, DeserializeSeed, Deserializer, EnumAccess, IgnoredAny, MapAccess, SeqAccess, Visitor,
};
use serde::Deserialize;
use serde_json::{Map, Number, Value};
use std::cell::Cell;
use std::fmt;

#[derive(Default)]
struct Budget {
    expanded: usize,
    reason: Option<&'static str>,
    contains_float: bool,
}

impl Budget {
    fn fail<E: de::Error>(&mut self, reason: &'static str) -> E {
        self.reason = Some(reason);
        E::custom(reason)
    }

    fn charge<E: de::Error>(&mut self, bytes: usize) -> Result<(), E> {
        self.expanded = self.expanded.saturating_add(bytes);
        if self.expanded > MAX_DOCUMENT_BYTES {
            return Err(self.fail("expanded document exceeds size limit"));
        }
        Ok(())
    }
}

struct Checked<'a> {
    budget: &'a mut Budget,
    depth: usize,
    key: bool,
}

impl<'de> DeserializeSeed<'de> for Checked<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Value, D::Error> {
        if self.depth > MAX_DEPTH {
            return Err(self.budget.fail("document exceeds nesting limit"));
        }
        self.budget.charge(1)?;
        // deserialize_any preserves scalar types; deserialize_string would
        // silently coerce numeric/boolean YAML mapping keys into strings.
        deserializer.deserialize_any(self)
    }
}

impl Checked<'_> {
    fn value_only<E: de::Error>(&mut self) -> Result<(), E> {
        if self.key {
            return Err(self.budget.fail("mapping keys must be strings"));
        }
        Ok(())
    }
}

impl<'de> Visitor<'de> for Checked<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("an unambiguous JSON-compatible YAML value")
    }

    fn visit_bool<E: de::Error>(mut self, value: bool) -> Result<Value, E> {
        self.value_only()?;
        Ok(Value::Bool(value))
    }

    fn visit_unit<E: de::Error>(mut self) -> Result<Value, E> {
        self.value_only()?;
        Ok(Value::Null)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Value, E> {
        self.budget.charge(value.len())?;
        Ok(Value::String(value.to_owned()))
    }

    fn visit_i64<E: de::Error>(mut self, value: i64) -> Result<Value, E> {
        self.value_only()?;
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E: de::Error>(mut self, value: u64) -> Result<Value, E> {
        self.value_only()?;
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E: de::Error>(mut self, value: f64) -> Result<Value, E> {
        self.value_only()?;
        self.budget.contains_float = true;
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| self.budget.fail("non-finite numbers are not JSON values"))
    }

    fn visit_seq<A: SeqAccess<'de>>(mut self, mut seq: A) -> Result<Value, A::Error> {
        self.value_only()?;
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(Checked {
            budget: self.budget,
            depth: self.depth + 1,
            key: false,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(mut self, mut map: A) -> Result<Value, A::Error> {
        self.value_only()?;
        let mut values = Map::new();
        while let Some(key) = map.next_key_seed(Checked {
            budget: self.budget,
            depth: self.depth + 1,
            key: true,
        })? {
            let Value::String(key) = key else {
                return Err(self.budget.fail("mapping keys must be strings"));
            };
            if key == "<<" {
                return Err(self.budget.fail("YAML merge keys are not supported"));
            }
            if values.contains_key(&key) {
                return Err(self.budget.fail("duplicate mapping key"));
            }
            let value = map.next_value_seed(Checked {
                budget: self.budget,
                depth: self.depth + 1,
                key: false,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }

    fn visit_enum<A: EnumAccess<'de>>(self, _: A) -> Result<Value, A::Error> {
        Err(self.budget.fail("custom YAML tags are not supported"))
    }
}

struct SourceValue<'a> {
    parsed: &'a Value,
    out_of_range: &'a Cell<bool>,
}

impl<'de> DeserializeSeed<'de> for SourceValue<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        match self.parsed {
            Value::Array(values) => deserializer.deserialize_seq(SourceSequence {
                values,
                out_of_range: self.out_of_range,
            }),
            Value::Object(values) => deserializer.deserialize_map(SourceMap {
                values,
                out_of_range: self.out_of_range,
            }),
            Value::Number(number) if number.is_f64() => {
                deserializer.deserialize_str(DecimalInteger(self.out_of_range))
            }
            _ => IgnoredAny::deserialize(deserializer).map(|_| ()),
        }
    }
}

struct SourceSequence<'a> {
    values: &'a [Value],
    out_of_range: &'a Cell<bool>,
}

impl<'de> Visitor<'de> for SourceSequence<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("the sequence from the checked YAML document")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<(), A::Error> {
        for value in self.values {
            if sequence
                .next_element_seed(SourceValue {
                    parsed: value,
                    out_of_range: self.out_of_range,
                })?
                .is_none()
            {
                return Err(de::Error::custom(
                    "YAML sequence changed during source check",
                ));
            }
        }
        if sequence.next_element::<IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(
                "YAML sequence changed during source check",
            ));
        }
        Ok(())
    }
}

struct SourceMap<'a> {
    values: &'a Map<String, Value>,
    out_of_range: &'a Cell<bool>,
}

impl<'de> Visitor<'de> for SourceMap<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("the mapping from the checked YAML document")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let mut entries = 0;
        while let Some(key) = map.next_key::<String>()? {
            let Some(value) = self.values.get(&key) else {
                return Err(de::Error::custom(
                    "YAML mapping changed during source check",
                ));
            };
            map.next_value_seed(SourceValue {
                parsed: value,
                out_of_range: self.out_of_range,
            })?;
            entries += 1;
        }
        if entries != self.values.len() {
            return Err(de::Error::custom(
                "YAML mapping changed during source check",
            ));
        }
        Ok(())
    }
}

struct DecimalInteger<'a>(&'a Cell<bool>);

impl<'de> Visitor<'de> for DecimalInteger<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a YAML scalar")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<(), E> {
        if is_unrepresentable_decimal_integer(value) {
            self.0.set(true);
            Err(E::custom("integer is outside the supported range"))
        } else {
            Ok(())
        }
    }
}

fn is_unrepresentable_decimal_integer(value: &str) -> bool {
    let (negative, digits) = if let Some(digits) = value.strip_prefix('-') {
        (true, digits)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    if negative {
        value.parse::<i128>().is_err()
    } else {
        digits.parse::<u128>().is_err()
    }
}

fn check_integer_literals(input: &str, parsed: &Value) -> Result<(), Error> {
    let out_of_range = Cell::new(false);
    let mut documents = serde_yaml_ng::Deserializer::from_str(input);
    let Some(document) = documents.next() else {
        return Err(Error::Document {
            reason: "expected one document",
            line: None,
            column: None,
        });
    };
    if let Err(error) = (SourceValue {
        parsed,
        out_of_range: &out_of_range,
    })
    .deserialize(document)
    {
        let location = error.location();
        return Err(Error::Document {
            reason: if out_of_range.get() {
                "integer is outside the supported range"
            } else {
                "could not inspect YAML source scalars"
            },
            line: location.as_ref().map(|location| location.line()),
            column: location.as_ref().map(|location| location.column()),
        });
    }
    Ok(())
}

pub(super) fn parse(input: &[u8]) -> Result<Value, Error> {
    let invalid = |reason| Error::Document {
        reason,
        line: None,
        column: None,
    };
    if input.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("document exceeds size limit"));
    }
    let input = std::str::from_utf8(input).map_err(|_| invalid("expected UTF-8 input"))?;
    let mut budget = Budget::default();
    let mut documents = serde_yaml_ng::Deserializer::from_str(input);
    let document = documents
        .next()
        .ok_or_else(|| invalid("expected one document"))?;
    let value = Checked {
        budget: &mut budget,
        depth: 0,
        key: false,
    }
    .deserialize(document)
    .map_err(|error| Error::Document {
        reason: budget.reason.unwrap_or("invalid YAML syntax or scalar"),
        line: error.location().map(|location| location.line()),
        column: error.location().map(|location| location.column()),
    })?;
    if documents.next().is_some() {
        return Err(invalid("expected exactly one document"));
    }
    // The checked value keeps the numeric type but not the source spelling.
    // Re-read only when floats exist and inspect those scalar nodes in context.
    if budget.contains_float {
        check_integer_literals(input, &value)?;
    }
    // Alias expansion has an incremental budget above. Also bound the exact
    // serialized size, including escaping, punctuation and numeric spellings.
    let encoded = serde_json::to_vec(&value).map_err(|_| invalid("invalid JSON value"))?;
    if encoded.len() > MAX_DOCUMENT_BYTES {
        return Err(invalid("expanded document exceeds size limit"));
    }
    Ok(value)
}
