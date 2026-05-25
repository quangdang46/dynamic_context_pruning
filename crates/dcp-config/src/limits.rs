//! `LimitValue` — the number-or-percent shape for context-limit fields.
//!
//! SPEC.md §10.2 defines `compress.maxContextLimit` and
//! `compress.minContextLimit` as `number | string`, where the string
//! must be of the form `"<n>%"` with `0 < n <= 100`. The same shape
//! applies to the per-model overrides in `compress.modelMaxLimits` and
//! `compress.modelMinLimits`.

use schemars::JsonSchema;
use schemars::schema::{InstanceType, Schema, SchemaObject, SubschemaValidation};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One entry of a context-limit field.
///
/// Either a raw token count (`>= 1000`) or a percent-of-model-limit
/// expression like `"80%"` (`> 0` and `<= 100`).
///
/// # Example
///
/// ```rust
/// use dcp_config::LimitValue;
/// let n = LimitValue::Number(100_000);
/// let p = LimitValue::Percent(80);
/// assert_eq!(serde_json::to_string(&n).unwrap(), "100000");
/// assert_eq!(serde_json::to_string(&p).unwrap(), "\"80%\"");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LimitValue {
    /// Raw numeric token count.
    Number(u64),
    /// Percent of the model's known context limit, in `1..=100`.
    Percent(u8),
}

impl LimitValue {
    /// Resolve this limit against an optional model context limit.
    ///
    /// SPEC.md §8.1: `Number(n)` returns `n`; `Percent(p)` returns
    /// `model_limit * p / 100`, falling back to `100_000` when no model
    /// limit is known.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_config::LimitValue;
    /// assert_eq!(LimitValue::Number(50_000).resolve(None), 50_000);
    /// assert_eq!(LimitValue::Percent(80).resolve(Some(200_000)), 160_000);
    /// assert_eq!(LimitValue::Percent(80).resolve(None), 100_000);
    /// ```
    pub fn resolve(self, model_limit: Option<u64>) -> u64 {
        match self {
            Self::Number(n) => n,
            Self::Percent(p) => match model_limit {
                Some(m) => m.saturating_mul(p as u64) / 100,
                None => 100_000,
            },
        }
    }

    /// Render in the canonical JSONC string form.
    pub fn render(self) -> String {
        match self {
            Self::Number(n) => n.to_string(),
            Self::Percent(p) => format!("{p}%"),
        }
    }
}

impl Serialize for LimitValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Number(n) => serializer.serialize_u64(*n),
            Self::Percent(p) => serializer.serialize_str(&format!("{p}%")),
        }
    }
}

impl<'de> Deserialize<'de> for LimitValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::{self, Visitor};
        use std::fmt;

        struct LimitVisitor;

        impl<'de> Visitor<'de> for LimitVisitor {
            type Value = LimitValue;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a number or a percent string like \"80%\"")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(LimitValue::Number(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    return Err(E::custom("limit value must be non-negative"));
                }
                Ok(LimitValue::Number(v as u64))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                if !v.is_finite() || v < 0.0 {
                    return Err(E::custom(
                        "limit value must be a finite non-negative number",
                    ));
                }
                // Round down to integer tokens.
                Ok(LimitValue::Number(v as u64))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                parse_percent(v).map(LimitValue::Percent).map_err(E::custom)
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                self.visit_str(&v)
            }
        }

        deserializer.deserialize_any(LimitVisitor)
    }
}

fn parse_percent(s: &str) -> Result<u8, String> {
    let trimmed = s.trim();
    let body = trimmed
        .strip_suffix('%')
        .ok_or_else(|| format!("limit string {trimmed:?} must end with '%'"))?;
    let n: u32 = body
        .trim()
        .parse()
        .map_err(|_| format!("limit string {trimmed:?} is not an integer percent"))?;
    if n == 0 {
        return Err("percent must be greater than 0%".into());
    }
    if n > 100 {
        return Err("percent must be less than or equal to 100%".into());
    }
    Ok(n as u8)
}

impl JsonSchema for LimitValue {
    fn schema_name() -> String {
        "LimitValue".into()
    }

    fn json_schema(_generator: &mut schemars::r#gen::SchemaGenerator) -> Schema {
        let number = SchemaObject {
            instance_type: Some(InstanceType::Integer.into()),
            format: Some("uint64".into()),
            ..Default::default()
        };
        let mut percent = SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            ..Default::default()
        };
        percent.string().pattern = Some(r"^\d{1,3}%$".into());
        Schema::Object(SchemaObject {
            subschemas: Some(Box::new(SubschemaValidation {
                any_of: Some(vec![Schema::Object(number), Schema::Object(percent)]),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_round_trip() {
        let v = LimitValue::Number(123_456);
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, "123456");
        let back: LimitValue = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn percent_round_trip() {
        let v = LimitValue::Percent(80);
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, "\"80%\"");
        let back: LimitValue = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
    }

    #[test]
    fn resolve_number_ignores_model_limit() {
        assert_eq!(LimitValue::Number(50).resolve(None), 50);
        assert_eq!(LimitValue::Number(50).resolve(Some(200_000)), 50);
    }

    #[test]
    fn resolve_percent_with_known_model_limit() {
        assert_eq!(LimitValue::Percent(50).resolve(Some(200_000)), 100_000);
        assert_eq!(LimitValue::Percent(80).resolve(Some(200_000)), 160_000);
        assert_eq!(LimitValue::Percent(100).resolve(Some(123)), 123);
    }

    #[test]
    fn resolve_percent_without_model_falls_back_to_100k() {
        assert_eq!(LimitValue::Percent(80).resolve(None), 100_000);
    }

    #[test]
    fn rejects_invalid_percent_strings() {
        assert!(serde_json::from_str::<LimitValue>("\"\"").is_err());
        assert!(serde_json::from_str::<LimitValue>("\"abc\"").is_err());
        assert!(serde_json::from_str::<LimitValue>("\"0%\"").is_err());
        assert!(serde_json::from_str::<LimitValue>("\"101%\"").is_err());
        assert!(serde_json::from_str::<LimitValue>("\"50\"").is_err());
    }

    #[test]
    fn float_value_floors_to_integer() {
        let v: LimitValue = serde_json::from_str("100000.7").unwrap();
        assert_eq!(v, LimitValue::Number(100_000));
    }

    #[test]
    fn render_matches_serde_form() {
        assert_eq!(LimitValue::Number(42).render(), "42");
        assert_eq!(LimitValue::Percent(75).render(), "75%");
    }

    #[test]
    fn negative_integer_is_rejected() {
        assert!(serde_json::from_str::<LimitValue>("-1").is_err());
    }

    #[test]
    fn schema_is_any_of() {
        let mut generator = schemars::r#gen::SchemaGenerator::default();
        let schema = LimitValue::json_schema(&mut generator);
        let v = serde_json::to_value(&schema).unwrap();
        assert!(v.get("anyOf").is_some(), "{v}");
    }
}
