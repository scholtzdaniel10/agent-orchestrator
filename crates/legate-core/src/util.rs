use serde_json::Value;

/// JSON integers, including `2.0`, which JavaScript treats as an integer.
pub(crate) fn json_i64(value: &Value) -> Option<i64> {
    if let Some(number) = value.as_i64() {
        return Some(number);
    }
    let number = value.as_f64()?;
    if number.is_finite()
        && number.fract() == 0.0
        && (i64::MIN as f64..=i64::MAX as f64).contains(&number)
    {
        return Some(number as i64);
    }
    None
}
