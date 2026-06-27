use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ast::{Expr, FuncArg, FunctionCall};
use crate::diagnostic::DaihonDiagnostic;
use crate::value::{DaihonValue, ValueType};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationMode {
    #[default]
    Strict,
    Permissive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionSpec {
    pub name: String,
    pub positional: Vec<ParamSpec>,
    pub named: BTreeMap<String, ParamSpec>,
    pub return_type: Option<ValueType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamSpec {
    pub name: Option<String>,
    pub ty: ParamType,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParamType {
    Any,
    Number,
    String,
    Boolean,
    BareWord,
}

#[derive(Debug, Clone, Default)]
pub struct FunctionRegistry {
    specs: BTreeMap<String, FunctionSpec>,
    mode: ValidationMode,
}

impl FunctionRegistry {
    pub fn new() -> Self {
        Self {
            specs: BTreeMap::new(),
            mode: ValidationMode::Strict,
        }
    }

    pub fn permissive() -> Self {
        Self {
            specs: BTreeMap::new(),
            mode: ValidationMode::Permissive,
        }
    }

    pub fn mode(&self) -> ValidationMode {
        self.mode
    }

    pub fn set_mode(&mut self, mode: ValidationMode) {
        self.mode = mode;
    }

    pub fn register(&mut self, spec: FunctionSpec) {
        self.specs.insert(spec.name.clone(), spec);
    }

    pub fn get(&self, name: &str) -> Option<&FunctionSpec> {
        self.specs.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.specs.contains_key(name)
    }

    pub fn specs(&self) -> &BTreeMap<String, FunctionSpec> {
        &self.specs
    }

    pub fn validate_call(&self, call: &FunctionCall, used_as_expr: bool) -> Vec<DaihonDiagnostic> {
        let Some(spec) = self.get(&call.name.value) else {
            return if self.mode == ValidationMode::Strict {
                vec![DaihonDiagnostic::error(
                    "E-DHN-SEM-040",
                    format!("未知の関数「{}」です。", call.name.value),
                    call.name.span,
                )
                .with_help("World Packの関数定義に追加するか、関数名の誤字を直してください。")]
            } else {
                Vec::new()
            };
        };

        let mut diagnostics = Vec::new();
        let required = spec
            .positional
            .iter()
            .filter(|param| param.required)
            .count();
        if call.positional.len() < required {
            diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SEM-041",
                    format!(
                        "関数「{}」の位置引数が不足しています。最低{}個必要です。",
                        spec.name, required
                    ),
                    call.span,
                )
                .with_help("不足している引数を追加してください。"),
            );
        }
        if call.positional.len() > spec.positional.len() {
            diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SEM-042",
                    format!(
                        "関数「{}」の位置引数が多すぎます。最大{}個です。",
                        spec.name,
                        spec.positional.len()
                    ),
                    call.span,
                )
                .with_help("余分な引数を削除するか、名前付き引数として定義してください。"),
            );
        }

        for (index, arg) in call.positional.iter().enumerate() {
            if let Some(param) = spec.positional.get(index) {
                if !param_accepts(param.ty, arg) {
                    diagnostics.push(DaihonDiagnostic::error(
                        "E-DHN-SEM-043",
                        format!(
                            "関数「{}」の{}番目の引数は {:?} が必要です。",
                            spec.name,
                            index + 1,
                            param.ty
                        ),
                        arg.span(),
                    ));
                }
            }
        }

        for (name, arg) in &call.named {
            match spec.named.get(name) {
                Some(param) if !param_accepts(param.ty, arg) => {
                    diagnostics.push(DaihonDiagnostic::error(
                        "E-DHN-SEM-044",
                        format!(
                            "関数「{}」の名前付き引数「{}」は {:?} が必要です。",
                            spec.name, name, param.ty
                        ),
                        arg.span(),
                    ))
                }
                Some(_) => {}
                None => diagnostics.push(
                    DaihonDiagnostic::error(
                        "E-DHN-SEM-045",
                        format!(
                            "関数「{}」に「{}」という名前付き引数はありません。",
                            spec.name, name
                        ),
                        arg.span(),
                    )
                    .with_help("関数定義の名前付き引数名を確認してください。"),
                ),
            }
        }

        for (name, param) in &spec.named {
            if param.required && !call.named.contains_key(name) {
                diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SEM-046",
                    format!(
                        "関数「{}」の名前付き引数「{}」が不足しています。",
                        spec.name, name
                    ),
                    call.span,
                ));
            }
        }

        if used_as_expr && spec.return_type.is_none() {
            diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SEM-047",
                    format!(
                        "関数「{}」は戻り値を返さないため、式の中では使えません。",
                        spec.name
                    ),
                    call.span,
                )
                .with_help("独立した行で呼び出すか、戻り値を持つ関数定義に変更してください。"),
            );
        }

        diagnostics
    }
}

fn param_accepts(ty: ParamType, arg: &FuncArg) -> bool {
    match ty {
        ParamType::Any => true,
        ParamType::BareWord => matches!(arg, FuncArg::BareWord(_)),
        ParamType::Number => {
            matches!(arg, FuncArg::Expr(Expr::Value(value)) if matches!(value.value, DaihonValue::Number(_)))
        }
        ParamType::String => match arg {
            FuncArg::BareWord(_) => true,
            FuncArg::Expr(Expr::Value(value)) => matches!(value.value, DaihonValue::String(_)),
            _ => false,
        },
        ParamType::Boolean => {
            matches!(arg, FuncArg::Expr(Expr::Value(value)) if matches!(value.value, DaihonValue::Boolean(_)))
        }
    }
}
