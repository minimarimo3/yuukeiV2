use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, FixedOffset, Timelike};
use serde::{Deserialize, Serialize};

use crate::diagnostic::DaihonRuntimeError;
use crate::span::{Span, Spanned};
use crate::value::{DaihonNumber, DaihonValue, ValueType};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum VariableRef {
    EventLocal {
        name: Spanned<String>,
    },
    Temporary {
        name: Spanned<String>,
    },
    Global {
        name: Spanned<String>,
    },
    Input {
        name: Spanned<String>,
    },
    Resident {
        actor: Spanned<String>,
        name: Spanned<String>,
    },
    Relation {
        subject: Spanned<String>,
        object: Spanned<String>,
        name: Spanned<String>,
    },
    Unsupported {
        scope: Spanned<String>,
        parts: Vec<Spanned<String>>,
    },
}

impl VariableRef {
    pub fn span(&self) -> Span {
        match self {
            Self::EventLocal { name }
            | Self::Temporary { name }
            | Self::Global { name }
            | Self::Input { name } => name.span,
            Self::Resident { actor, name } => actor.span.join(name.span),
            Self::Relation {
                subject,
                object,
                name,
            } => subject.span.join(object.span).join(name.span),
            Self::Unsupported { scope, parts } => parts
                .iter()
                .fold(scope.span, |span, part| span.join(part.span)),
        }
    }

    pub fn display_name(&self) -> String {
        match self {
            Self::EventLocal { name } | Self::Temporary { name } => name.value.clone(),
            Self::Global { name } => format!("全体#{}", name.value),
            Self::Input { name } => format!("入力#{}", name.value),
            Self::Resident { actor, name } => format!("住人#{}#{}", actor.value, name.value),
            Self::Relation {
                subject,
                object,
                name,
            } => format!("関係#{}#{}#{}", subject.value, object.value, name.value),
            Self::Unsupported { scope, parts } => {
                let rest = parts
                    .iter()
                    .map(|part| part.value.as_str())
                    .collect::<Vec<_>>()
                    .join("#");
                if rest.is_empty() {
                    scope.value.clone()
                } else {
                    format!("{}#{rest}", scope.value)
                }
            }
        }
    }

    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::Input { .. })
    }
}

pub trait VariableStore {
    fn is_defined(&self, reference: &VariableRef) -> bool;

    fn get_value(&self, reference: &VariableRef) -> Result<DaihonValue, DaihonRuntimeError>;

    fn set_value(
        &mut self,
        reference: &VariableRef,
        value: DaihonValue,
    ) -> Result<(), DaihonRuntimeError>;

    fn set_default_value(
        &mut self,
        reference: &VariableRef,
        value: DaihonValue,
    ) -> Result<(), DaihonRuntimeError>;

    fn clear_temporary_variables(&mut self);
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryVariableStore {
    values: BTreeMap<String, DaihonValue>,
    input_values: BTreeMap<String, DaihonValue>,
}

impl InMemoryVariableStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_input(mut self, name: impl Into<String>, value: DaihonValue) -> Self {
        self.input_values.insert(name.into(), value);
        self
    }

    pub fn values(&self) -> &BTreeMap<String, DaihonValue> {
        &self.values
    }

    fn key(reference: &VariableRef) -> String {
        reference.display_name()
    }

    fn check_type(
        reference: &VariableRef,
        existing: &DaihonValue,
        value: &DaihonValue,
    ) -> Result<(), DaihonRuntimeError> {
        let old_ty = existing.value_type();
        let new_ty = value.value_type();
        if old_ty != ValueType::None && new_ty != ValueType::None && old_ty != new_ty {
            return Err(DaihonRuntimeError::new(
                "E-DHN-RUN-020",
                format!(
                    "{} は既に {:?} 型です。{:?} 型の値は代入できません。",
                    reference.display_name(),
                    old_ty,
                    new_ty
                ),
                reference.span(),
            ));
        }
        Ok(())
    }
}

impl VariableStore for InMemoryVariableStore {
    fn is_defined(&self, reference: &VariableRef) -> bool {
        match reference {
            VariableRef::Input { name } => self.input_values.contains_key(&name.value),
            VariableRef::Unsupported { .. } => false,
            _ => self.values.contains_key(&Self::key(reference)),
        }
    }

    fn get_value(&self, reference: &VariableRef) -> Result<DaihonValue, DaihonRuntimeError> {
        match reference {
            VariableRef::Input { name } => {
                self.input_values.get(&name.value).cloned().ok_or_else(|| {
                    DaihonRuntimeError::new(
                        "E-DHN-RUN-010",
                        format!("入力#{} は渡されていません。", name.value),
                        name.span,
                    )
                })
            }
            VariableRef::Unsupported { .. } => Err(DaihonRuntimeError::new(
                "E-DHN-RUN-011",
                format!(
                    "{} は使用できない変数スコープです。",
                    reference.display_name()
                ),
                reference.span(),
            )),
            _ => self
                .values
                .get(&Self::key(reference))
                .cloned()
                .ok_or_else(|| {
                    DaihonRuntimeError::new(
                        "E-DHN-RUN-012",
                        format!("{} はまだ定義されていません。", reference.display_name()),
                        reference.span(),
                    )
                }),
        }
    }

    fn set_value(
        &mut self,
        reference: &VariableRef,
        value: DaihonValue,
    ) -> Result<(), DaihonRuntimeError> {
        if reference.is_read_only() {
            return Err(DaihonRuntimeError::new(
                "E-DHN-RUN-013",
                format!("{} には代入できません。", reference.display_name()),
                reference.span(),
            ));
        }
        let key = Self::key(reference);
        if let Some(existing) = self.values.get(&key) {
            Self::check_type(reference, existing, &value)?;
        }
        self.values.insert(key, value);
        Ok(())
    }

    fn set_default_value(
        &mut self,
        reference: &VariableRef,
        value: DaihonValue,
    ) -> Result<(), DaihonRuntimeError> {
        if reference.is_read_only() {
            return Err(DaihonRuntimeError::new(
                "E-DHN-RUN-014",
                format!(
                    "{} は読み取り専用なので初期値を設定できません。",
                    reference.display_name()
                ),
                reference.span(),
            ));
        }
        let key = Self::key(reference);
        self.values.entry(key).or_insert(value);
        Ok(())
    }

    fn clear_temporary_variables(&mut self) {
        self.values.retain(|key, _| !key.starts_with('_'));
    }
}

pub fn builtin_time_value(
    reference: &VariableRef,
    now: DateTime<FixedOffset>,
) -> Option<DaihonValue> {
    let name = match reference {
        VariableRef::EventLocal { name } => name.value.as_str(),
        _ => return None,
    };

    let number = |value: i64| DaihonValue::Number(DaihonNumber::Integer(value));
    match name {
        "年" => Some(number(now.year() as i64)),
        "月" => Some(number(now.month() as i64)),
        "日" => Some(number(now.day() as i64)),
        "曜日" => {
            let value = match now.weekday().number_from_monday() {
                1 => "月",
                2 => "火",
                3 => "水",
                4 => "木",
                5 => "金",
                6 => "土",
                _ => "日",
            };
            Some(DaihonValue::String(value.to_owned()))
        }
        "週" => Some(number(((now.day() - 1) / 7 + 1) as i64)),
        "時" => Some(number(now.hour() as i64)),
        "分" => Some(number(now.minute() as i64)),
        "秒" => Some(number(now.second() as i64)),
        "ミリ秒" => Some(number(now.timestamp_subsec_millis() as i64)),
        _ => None,
    }
}
