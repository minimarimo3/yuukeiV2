use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueType {
    None,
    Number,
    String,
    Boolean,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DaihonNumber {
    Integer(i64),
    Float(f64),
}

impl PartialEq for DaihonNumber {
    fn eq(&self, other: &Self) -> bool {
        self.as_f64() == other.as_f64()
    }
}

impl DaihonNumber {
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Integer(value) => value as f64,
            Self::Float(value) => value,
        }
    }

    pub fn is_integer(self) -> bool {
        matches!(self, Self::Integer(_))
    }

    pub fn checked_add(self, other: Self) -> Self {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => Self::Integer(left + right),
            (left, right) => Self::Float(left.as_f64() + right.as_f64()),
        }
    }

    pub fn checked_sub(self, other: Self) -> Self {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => Self::Integer(left - right),
            (left, right) => Self::Float(left.as_f64() - right.as_f64()),
        }
    }

    pub fn checked_mul(self, other: Self) -> Self {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => Self::Integer(left * right),
            (left, right) => Self::Float(left.as_f64() * right.as_f64()),
        }
    }

    pub fn checked_div(self, other: Self) -> Option<Self> {
        if other.as_f64() == 0.0 {
            return None;
        }
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => Some(Self::Integer(left / right)),
            (left, right) => Some(Self::Float(left.as_f64() / right.as_f64())),
        }
    }

    pub fn checked_rem(self, other: Self) -> Option<Self> {
        if other.as_f64() == 0.0 {
            return None;
        }
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => Some(Self::Integer(left % right)),
            (left, right) => Some(Self::Float(left.as_f64() % right.as_f64())),
        }
    }
}

impl fmt::Display for DaihonNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Integer(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DaihonValue {
    None,
    Number(DaihonNumber),
    String(String),
    Boolean(bool),
}

impl DaihonValue {
    pub fn value_type(&self) -> ValueType {
        match self {
            Self::None => ValueType::None,
            Self::Number(_) => ValueType::Number,
            Self::String(_) => ValueType::String,
            Self::Boolean(_) => ValueType::Boolean,
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::None => String::new(),
            Self::Number(number) => number.to_string(),
            Self::String(value) => value.clone(),
            Self::Boolean(true) => "はい".to_owned(),
            Self::Boolean(false) => "いいえ".to_owned(),
        }
    }

    pub fn as_number(&self) -> Option<DaihonNumber> {
        match self {
            Self::Number(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub fn compare_same_type(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Number(left), Self::Number(right)) => left.as_f64().partial_cmp(&right.as_f64()),
            (Self::String(left), Self::String(right)) => Some(left.cmp(right)),
            (Self::Boolean(left), Self::Boolean(right)) => Some(left.cmp(right)),
            (Self::None, Self::None) => Some(Ordering::Equal),
            _ => None,
        }
    }
}
