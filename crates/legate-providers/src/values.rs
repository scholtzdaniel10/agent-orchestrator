use serde_json::{Map, Number, Value};

pub(crate) fn string_arg<'a>(input: &'a Value, keys: &[&str]) -> Option<&'a str> {
    let obj = input.as_object()?;
    for key in keys {
        if let Some(Value::String(value)) = obj.get(*key)
            && !value.is_empty()
        {
            return Some(value);
        }
    }
    None
}

pub(crate) fn number_f64(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

pub(crate) fn epoch_secs(value: f64) -> Option<i64> {
    if value.is_finite()
        && value.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&value)
    {
        Some(value as i64)
    } else {
        None
    }
}

pub(crate) fn token_total(input: f64, output: f64) -> Option<i64> {
    let sum = input + output;
    if sum.is_finite() && (i64::MIN as f64..=i64::MAX as f64).contains(&sum) {
        Some(sum.round() as i64)
    } else {
        None
    }
}

/// `String(number)` for a finite JSON number: integers print without a fraction.
pub(crate) fn js_number(value: f64) -> String {
    if let Some(whole) = epoch_secs(value) {
        whole.to_string()
    } else if let Some(number) = Number::from_f64(value) {
        number.to_string()
    } else {
        String::new()
    }
}

pub(crate) fn empty_object() -> Value {
    Value::Object(Map::new())
}
