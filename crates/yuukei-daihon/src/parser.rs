use std::collections::BTreeMap;
use std::time::Duration;

use chumsky::prelude::*;

use crate::ast::*;
use crate::diagnostic::DaihonDiagnostic;
use crate::lexer::{lex_source, normalize_char, normalize_syntax};
use crate::span::{Span, Spanned};
use crate::value::{DaihonNumber, DaihonValue};
use crate::variable::VariableRef;

#[derive(Debug, Clone)]
struct LogicalLine {
    text: String,
    span: Span,
}

#[derive(Debug, Clone, PartialEq)]
enum ExprTokenKind {
    Number(String),
    String(String),
    Bool(bool),
    Ident(String),
    Function(FunctionCall),
    Time(TimeOfDay),
    Op(String),
    LParen,
    RParen,
}

#[derive(Debug, Clone, PartialEq)]
struct ExprToken {
    kind: ExprTokenKind,
    span: Span,
}

pub fn parse_script(source: &str) -> Result<Script, Vec<DaihonDiagnostic>> {
    lex_source(source)?;
    let _ = chumsky_source_probe().parse(source);

    let lines = logical_lines(source);
    let mut parser = DaihonParser {
        lines,
        index: 0,
        diagnostics: Vec::new(),
    };
    let script = parser.parse_script();
    if parser.diagnostics.is_empty() {
        script.ok_or_else(|| {
            vec![DaihonDiagnostic::error(
                "E-DHN-SYN-001",
                "台本が空です。## イベント名 から始めてください。",
                Span::empty(),
            )]
        })
    } else {
        Err(parser.diagnostics)
    }
}

fn chumsky_source_probe() -> impl Parser<char, Vec<char>, Error = Simple<char>> {
    any().repeated().then_ignore(end())
}

struct DaihonParser {
    lines: Vec<LogicalLine>,
    index: usize,
    diagnostics: Vec<DaihonDiagnostic>,
}

impl DaihonParser {
    fn parse_script(&mut self) -> Option<Script> {
        self.skip_empty();
        let event_line = self.next_line()?;
        let Some(event_name) = parse_header(&event_line, "##") else {
            self.diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SYN-001",
                    "イベントヘッダーがありません。",
                    event_line.span,
                )
                .with_help("台本の先頭に ## イベント名 を書いてください。"),
            );
            return None;
        };
        if event_name.value.trim().is_empty() {
            self.diagnostics.push(DaihonDiagnostic::error(
                "E-DHN-SEM-001",
                "イベント名が空です。",
                event_line.span,
            ));
        }

        let mut preconditions = Vec::new();
        let mut defaults = Vec::new();

        self.skip_empty();
        if self.current_is_section("前提条件") {
            self.index += 1;
            while let Some(line) = self.peek_line() {
                if is_section_start(&line.text) || is_scene_header(&line.text) {
                    break;
                }
                if line.text.trim().is_empty() {
                    self.index += 1;
                    continue;
                }
                let owned = self.next_line().unwrap();
                match self.parse_precondition_line(&owned) {
                    Some(stmt) => preconditions.push(stmt),
                    None => self.diagnostics.push(DaihonDiagnostic::error(
                        "E-DHN-SYN-010",
                        "前提条件には 条件付き →おわり だけを書けます。",
                        owned.span,
                    )),
                }
            }
        }

        self.skip_empty();
        if self.current_is_section("初期値") {
            self.index += 1;
            while let Some(line) = self.peek_line() {
                if is_scene_header(&line.text) {
                    break;
                }
                if line.text.trim().is_empty() {
                    self.index += 1;
                    continue;
                }
                let owned = self.next_line().unwrap();
                if let Some(assignment) = parse_assignment(&owned, &mut self.diagnostics) {
                    defaults.push(assignment);
                } else {
                    self.diagnostics.push(DaihonDiagnostic::error(
                        "E-DHN-SYN-011",
                        "初期値ブロックには代入だけを書けます。",
                        owned.span,
                    ));
                }
            }
        }

        let mut scenes = Vec::new();
        while self.index < self.lines.len() {
            self.skip_empty();
            if self.index >= self.lines.len() {
                break;
            }
            if let Some(scene) = self.parse_scene() {
                scenes.push(scene);
            } else {
                self.index += 1;
            }
        }

        if scenes.is_empty() {
            self.diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SEM-003",
                    "シーンが1つもありません。",
                    event_line.span,
                )
                .with_help("### シーン名 を少なくとも1つ追加してください。"),
            );
        }

        let span = event_line.span.join(
            scenes
                .last()
                .map(|scene| scene.span)
                .unwrap_or(event_line.span),
        );
        Some(Script {
            event: Event {
                name: event_name,
                preconditions,
                defaults,
                scenes,
                span,
            },
            span,
        })
    }

    fn parse_scene(&mut self) -> Option<Scene> {
        let header = self.next_line()?;
        let Some(name) = parse_header(&header, "###") else {
            self.diagnostics.push(DaihonDiagnostic::error(
                "E-DHN-SYN-020",
                "シーンヘッダーが必要です。",
                header.span,
            ));
            return None;
        };
        if name.value.trim().is_empty() {
            self.diagnostics.push(DaihonDiagnostic::error(
                "E-DHN-SEM-004",
                "シーン名が空です。",
                header.span,
            ));
        }

        let metadata = self.parse_scene_metadata();
        let statements = self.parse_stmt_list_until_scene();
        let end_span = statements.last().map(stmt_span).unwrap_or(
            metadata
                .speaker
                .as_ref()
                .map(|s| s.span)
                .unwrap_or(header.span),
        );
        Some(Scene {
            name,
            metadata,
            statements,
            span: header.span.join(end_span),
        })
    }

    fn parse_scene_metadata(&mut self) -> SceneMetadata {
        let mut metadata = SceneMetadata::default();
        while let Some(line) = self.peek_line() {
            let normalized = normalize_line_head(&line.text);
            let Some((key, value)) = split_metadata_line(&normalized) else {
                break;
            };
            if !matches!(
                key.as_str(),
                "合図" | "条件" | "優先度" | "重み" | "クールダウン" | "話者"
            ) {
                if suggest_metadata_key(&key).is_some() {
                    let line = self.next_line().unwrap();
                    let key_span = span_for_substr(&line, &key);
                    metadata
                        .raw
                        .unknown_metadata_keys
                        .push(Spanned::new(key, key_span));
                    continue;
                }
                break;
            }
            let line = self.next_line().unwrap();
            let value_span = span_for_substr(&line, value);
            match key.as_str() {
                "合図" => {
                    let raw = Spanned::new(value.trim().to_owned(), value_span);
                    metadata.raw.signal_used_and = value.contains("かつ");
                    metadata.raw.signal_text = Some(raw);
                    for item in value.split("または") {
                        let trimmed = item.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let event_name = trimmed
                            .trim_start_matches('@')
                            .trim_start_matches('＠')
                            .trim()
                            .to_owned();
                        metadata
                            .signals
                            .push(SystemEvent::new(normalize_syntax(&event_name), value_span));
                    }
                }
                "条件" => {
                    metadata.raw.condition_had_marker = value.trim_start().starts_with('※');
                    metadata.raw.condition_text =
                        Some(Spanned::new(value.trim().to_owned(), value_span));
                    let cond_text = trim_condition_wrapper(value.trim().trim_start_matches('※'));
                    match parse_condition_expr(cond_text, value_span) {
                        Ok(expr) => metadata.condition = Some(expr),
                        Err(diag) => self.diagnostics.push(diag),
                    }
                }
                "優先度" => {
                    metadata.raw.priority_text =
                        Some(Spanned::new(value.trim().to_owned(), value_span));
                    if let Ok(value) = normalize_syntax(value.trim()).parse::<i32>() {
                        metadata.priority = value;
                    }
                }
                "重み" => {
                    metadata.raw.weight_text =
                        Some(Spanned::new(value.trim().to_owned(), value_span));
                    if let Ok(value) = normalize_syntax(value.trim()).parse::<u32>() {
                        metadata.weight = value;
                    } else {
                        metadata.weight = 0;
                    }
                }
                "クールダウン" => {
                    metadata.raw.cooldown_text =
                        Some(Spanned::new(value.trim().to_owned(), value_span));
                    metadata.cooldown = parse_duration(value.trim());
                }
                "話者" => {
                    metadata.speaker = Some(Spanned::new(value.trim().to_owned(), value_span));
                }
                _ => {}
            }
        }
        metadata
    }

    fn parse_stmt_list_until_scene(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();
        while let Some(line) = self.peek_line() {
            if is_scene_header(&line.text) {
                break;
            }
            if line.text.trim().is_empty() {
                self.index += 1;
                continue;
            }
            if normalize_syntax(line.text.trim()) == "おわり" {
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SYN-030",
                    "対応する条件ブロックがない おわり があります。",
                    line.span,
                ));
                self.index += 1;
                continue;
            }
            if let Some(stmt) = self.parse_stmt() {
                statements.push(stmt);
            } else {
                self.index += 1;
            }
        }
        statements
    }

    fn parse_stmt_list_until_branch_end(&mut self) -> Vec<Stmt> {
        let mut statements = Vec::new();
        while let Some(line) = self.peek_line() {
            let normalized = normalize_syntax(line.text.trim());
            if normalized == "おわり"
                || normalized.starts_with("※あるいは")
                || normalized.starts_with("※それ以外")
            {
                break;
            }
            if line.text.trim().is_empty() {
                self.index += 1;
                continue;
            }
            if let Some(stmt) = self.parse_stmt() {
                statements.push(stmt);
            } else {
                self.index += 1;
            }
        }
        statements
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        let line = self.next_line()?;
        parse_stmt_from_line(&line, self)
    }

    fn parse_precondition_line(&mut self, line: &LogicalLine) -> Option<ConditionalStmt> {
        let normalized = normalize_syntax(line.text.trim());
        if !normalized.starts_with('※') {
            return None;
        }
        let (cond, rest) = split_condition_prefix(&line.text, line.span)?;
        let rest_norm = normalize_syntax(rest.trim());
        if rest_norm != "→おわり" {
            return None;
        }
        Some(ConditionalStmt {
            condition: cond,
            action: Box::new(Stmt::Jump(JumpTarget::EndEvent { span: line.span })),
            span: line.span,
        })
    }

    fn skip_empty(&mut self) {
        while matches!(self.peek_line(), Some(line) if line.text.trim().is_empty()) {
            self.index += 1;
        }
    }

    fn peek_line(&self) -> Option<&LogicalLine> {
        self.lines.get(self.index)
    }

    fn next_line(&mut self) -> Option<LogicalLine> {
        let line = self.lines.get(self.index).cloned();
        if line.is_some() {
            self.index += 1;
        }
        line
    }

    fn current_is_section(&self, key: &str) -> bool {
        self.peek_line().is_some_and(|line| {
            let normalized = normalize_line_head(&line.text);
            split_metadata_line(&normalized)
                .map(|(actual, _)| actual == key)
                .unwrap_or(false)
        })
    }
}

fn parse_stmt_from_line(line: &LogicalLine, parser: &mut DaihonParser) -> Option<Stmt> {
    let trimmed = line.text.trim();
    if trimmed.starts_with('※') {
        return parser.parse_conditional_from_first_line(line.clone());
    }
    if trimmed.starts_with('→') {
        return Some(Stmt::Jump(parse_jump(trimmed, line.span)));
    }
    if let Some((speaker, rest)) = split_speaker_prefix(trimmed) {
        let speaker_span = span_for_substr(line, speaker);
        let rest_line = LogicalLine {
            text: rest.trim().to_owned(),
            span: span_for_substr(line, rest),
        };
        if let Some(display) = parse_display_line(&rest_line, &mut parser.diagnostics) {
            return Some(Stmt::SpeakerDisplay {
                speaker: Spanned::new(speaker.trim().to_owned(), speaker_span),
                display,
            });
        }
        parser.diagnostics.push(DaihonDiagnostic::error(
            "E-DHN-SEM-060",
            "話者prefixを付けられるのは表示列だけです。",
            speaker_span,
        ));
        return None;
    }
    if let Some(display) = parse_display_line(line, &mut parser.diagnostics) {
        return Some(Stmt::Display(display));
    }
    if let Some(assignment) = parse_assignment(line, &mut parser.diagnostics) {
        return Some(Stmt::Assignment(Box::new(assignment)));
    }
    let help = if split_metadata_line(&normalize_line_head(&line.text))
        .map(|(key, _)| is_known_metadata_key(&key))
        .unwrap_or(false)
    {
        "合図・条件などのメタデータ行はシーン見出しの直後にまとめて書いてください。"
    } else {
        "セリフは 「」 で囲んでください。地の文(裸のテキスト行)は書けません。セリフ、関数呼び出し、代入、ジャンプ、条件ブロックのいずれかを書いてください。"
    };
    parser.diagnostics.push(
        DaihonDiagnostic::error("E-DHN-SYN-031", "文として解釈できませんでした。", line.span)
            .with_help(help),
    );
    None
}

impl DaihonParser {
    fn parse_conditional_from_first_line(&mut self, first_line: LogicalLine) -> Option<Stmt> {
        let normalized = normalize_syntax(first_line.text.trim());
        if normalized.starts_with("※あるいは") || normalized.starts_with("※それ以外") {
            self.diagnostics.push(DaihonDiagnostic::error(
                "E-DHN-SYN-040",
                "※あるいは / ※それ以外 は、直前の条件ブロックの中でだけ使えます。",
                first_line.span,
            ));
            return None;
        }

        let (condition, rest) = match split_condition_prefix(&first_line.text, first_line.span) {
            Some(value) => value,
            None => {
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SYN-041",
                    "条件式を読み取れませんでした。",
                    first_line.span,
                ));
                return None;
            }
        };
        let rest_trimmed = trim_nara(rest.trim());
        if normalize_syntax(rest_trimmed).starts_with(':') {
            let mut branches = vec![ConditionalBranch {
                condition,
                statements: self.parse_stmt_list_until_branch_end(),
                span: first_line.span,
            }];
            let mut else_branch = None;
            loop {
                let Some(line) = self.peek_line().cloned() else {
                    self.diagnostics.push(DaihonDiagnostic::error(
                        "E-DHN-SYN-042",
                        "条件ブロックの おわり が不足しています。",
                        first_line.span,
                    ));
                    break;
                };
                let normalized = normalize_syntax(line.text.trim());
                if normalized == "おわり" {
                    self.index += 1;
                    break;
                }
                if normalized.starts_with("※あるいは") {
                    self.index += 1;
                    if else_branch.is_some() {
                        self.diagnostics.push(DaihonDiagnostic::error(
                            "E-DHN-SEM-032",
                            "※それ以外 の後に ※あるいは は書けません。",
                            line.span,
                        ));
                    }
                    let after = line
                        .text
                        .trim()
                        .trim_start_matches('※')
                        .trim_start_matches("あるいは");
                    let (cond, rest) = match split_paren_expr(after, line.span) {
                        Some(value) => value,
                        None => {
                            self.diagnostics.push(DaihonDiagnostic::error(
                                "E-DHN-SYN-043",
                                "※あるいは の条件式を読み取れませんでした。",
                                line.span,
                            ));
                            continue;
                        }
                    };
                    if !normalize_syntax(trim_nara(rest.trim())).starts_with(':') {
                        self.diagnostics.push(DaihonDiagnostic::error(
                            "E-DHN-SYN-044",
                            "※あるいは はブロック記法で : が必要です。",
                            line.span,
                        ));
                    }
                    branches.push(ConditionalBranch {
                        condition: cond,
                        statements: self.parse_stmt_list_until_branch_end(),
                        span: line.span,
                    });
                    continue;
                }
                if normalized.starts_with("※それ以外") {
                    self.index += 1;
                    if else_branch.is_some() {
                        self.diagnostics.push(DaihonDiagnostic::error(
                            "E-DHN-SEM-033",
                            "※それ以外 が複数あります。",
                            line.span,
                        ));
                    }
                    else_branch = Some(self.parse_stmt_list_until_branch_end());
                    continue;
                }
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SYN-045",
                    "条件ブロックの終端を読み取れませんでした。",
                    line.span,
                ));
                self.index += 1;
            }
            Some(Stmt::Conditional(ConditionalBlock {
                branches,
                else_branch,
                span: first_line.span,
                one_line: false,
            }))
        } else {
            let action_line = LogicalLine {
                text: rest_trimmed.to_owned(),
                span: span_for_substr(&first_line, rest_trimmed),
            };
            let Some(action) = parse_single_action(&action_line, &mut self.diagnostics) else {
                self.diagnostics.push(DaihonDiagnostic::error(
                    "E-DHN-SYN-046",
                    "条件付き1行記法のアクションを読み取れませんでした。",
                    first_line.span,
                ));
                return None;
            };
            Some(Stmt::Conditional(ConditionalBlock {
                branches: vec![ConditionalBranch {
                    condition,
                    statements: vec![action],
                    span: first_line.span,
                }],
                else_branch: None,
                span: first_line.span,
                one_line: true,
            }))
        }
    }
}

fn parse_single_action(
    line: &LogicalLine,
    diagnostics: &mut Vec<DaihonDiagnostic>,
) -> Option<Stmt> {
    if line.text.trim().starts_with('→') {
        return Some(Stmt::Jump(parse_jump(line.text.trim(), line.span)));
    }
    if let Some(display) = parse_display_line(line, diagnostics) {
        if display.parts.len() > 1 {
            diagnostics.push(
                DaihonDiagnostic::error(
                    "E-DHN-SEM-034",
                    "条件付き1行記法ではアクションは1つだけです。",
                    display.span,
                )
                .with_help(
                    "複数の表示や関数を条件付きにしたい場合はブロック記法を使ってください。",
                ),
            );
        }
        return Some(Stmt::Display(display));
    }
    parse_assignment(line, diagnostics).map(|assignment| Stmt::Assignment(Box::new(assignment)))
}

fn parse_header(line: &LogicalLine, marker: &str) -> Option<Spanned<String>> {
    let normalized = normalize_syntax(line.text.trim_start());
    if !normalized.starts_with(marker) {
        return None;
    }
    let name = normalized[marker.len()..].trim().to_owned();
    Some(Spanned::new(name, line.span))
}

fn parse_jump(text: &str, span: Span) -> JumpTarget {
    let target = normalize_syntax(text.trim().trim_start_matches('→'))
        .trim()
        .to_owned();
    match target.as_str() {
        "おわり" => JumpTarget::EndEvent { span },
        "シーンおわり" => JumpTarget::EndScene { span },
        _ => JumpTarget::Scene {
            name: Spanned::new(target, span),
        },
    }
}

fn parse_assignment(
    line: &LogicalLine,
    diagnostics: &mut Vec<DaihonDiagnostic>,
) -> Option<Assignment> {
    let index = find_assignment_index(&line.text)?;
    let (left, right_with_eq) = line.text.split_at(index);
    let right = &right_with_eq['='.len_utf8()..];
    let target_span = span_for_substr(line, left);
    let target = parse_variable_ref(left.trim(), target_span);
    match parse_arith_expr(right.trim(), span_for_substr(line, right.trim())) {
        Ok(value) => Some(Assignment {
            target,
            value,
            span: line.span,
        }),
        Err(diag) => {
            diagnostics.push(diag);
            None
        }
    }
}

fn parse_display_line(
    line: &LogicalLine,
    diagnostics: &mut Vec<DaihonDiagnostic>,
) -> Option<DisplayLine> {
    let mut parts = Vec::new();
    let mut byte = 0usize;
    let text = line.text.trim();
    let trim_offset = line.text.find(text).unwrap_or(0);
    byte += trim_offset;
    while byte < line.text.len() {
        let rest = &line.text[byte..];
        if rest.trim_start().len() != rest.len() {
            byte += rest.len() - rest.trim_start().len();
            continue;
        }
        let ch = rest.chars().next()?;
        if ch == '「' {
            match parse_dialogue_at(&line.text, byte, line.span) {
                Ok((dialogue, next)) => {
                    byte = next;
                    parts.push(DisplayPart::Dialogue(dialogue));
                }
                Err(diag) => {
                    diagnostics.push(diag);
                    return None;
                }
            }
        } else if ch == '＜' {
            match parse_function_call_at(&line.text, byte, line.span) {
                Ok((function, next)) => {
                    byte = next;
                    parts.push(DisplayPart::FunctionCall(function));
                }
                Err(diag) => {
                    diagnostics.push(diag);
                    return None;
                }
            }
        } else {
            return None;
        }
    }
    if parts.is_empty() {
        None
    } else {
        let span = parts
            .iter()
            .fold(parts[0].span(), |span, part| span.join(part.span()));
        Some(DisplayLine { parts, span })
    }
}

fn parse_dialogue_at(
    source: &str,
    start_byte: usize,
    base_span: Span,
) -> Result<(Dialogue, usize), DaihonDiagnostic> {
    let mut parts = Vec::new();
    let mut byte = start_byte + '「'.len_utf8();
    let mut text_start = byte;
    let mut text = String::new();
    while byte < source.len() {
        let ch = source[byte..].chars().next().unwrap();
        if ch == '」' {
            if source[byte + ch.len_utf8()..].starts_with('」') {
                text.push('」');
                byte += ch.len_utf8() * 2;
                continue;
            }
            if !text.is_empty() {
                parts.push(DialoguePart::Text(Spanned::new(
                    text.clone(),
                    relative_span(base_span, source, text_start, byte),
                )));
            }
            let end = byte + ch.len_utf8();
            return Ok((
                Dialogue {
                    parts,
                    span: relative_span(base_span, source, start_byte, end),
                },
                end,
            ));
        }
        if ch == '「' && source[byte + ch.len_utf8()..].starts_with('「') {
            text.push('「');
            byte += ch.len_utf8() * 2;
            continue;
        }
        if ch == '＜' {
            if source[byte + ch.len_utf8()..].starts_with('＜') {
                text.push('＜');
                byte += ch.len_utf8() * 2;
                continue;
            }
            if !text.is_empty() {
                parts.push(DialoguePart::Text(Spanned::new(
                    text.clone(),
                    relative_span(base_span, source, text_start, byte),
                )));
                text.clear();
            }
            let (function, next) = parse_function_call_at(source, byte, base_span)?;
            parts.push(DialoguePart::Embed(function));
            byte = next;
            text_start = byte;
            continue;
        }
        if ch == '＞' && source[byte + ch.len_utf8()..].starts_with('＞') {
            text.push('＞');
            byte += ch.len_utf8() * 2;
            continue;
        }
        text.push(ch);
        byte += ch.len_utf8();
    }
    Err(DaihonDiagnostic::error(
        "E-DHN-LEX-001",
        "セリフが閉じられていません。",
        relative_span(base_span, source, start_byte, source.len()),
    ))
}

fn parse_function_call_at(
    source: &str,
    start_byte: usize,
    base_span: Span,
) -> Result<(FunctionCall, usize), DaihonDiagnostic> {
    let mut byte = start_byte + '＜'.len_utf8();
    let content_start = byte;
    let mut depth = 1usize;
    let mut in_string = false;
    while byte < source.len() {
        let ch = source[byte..].chars().next().unwrap();
        if in_string {
            if ch == '」' {
                if source[byte + ch.len_utf8()..].starts_with('」') {
                    byte += ch.len_utf8() * 2;
                    continue;
                }
                in_string = false;
            }
            byte += ch.len_utf8();
            continue;
        }
        match ch {
            '「' => {
                in_string = true;
                byte += ch.len_utf8();
            }
            '＜' => {
                depth += 1;
                byte += ch.len_utf8();
            }
            '＞' => {
                depth -= 1;
                if depth == 0 {
                    let content = &source[content_start..byte];
                    let end = byte + ch.len_utf8();
                    let call = parse_function_content(
                        content,
                        relative_span(base_span, source, start_byte, end),
                    )?;
                    return Ok((call, end));
                }
                byte += ch.len_utf8();
            }
            _ => byte += ch.len_utf8(),
        }
    }
    Err(DaihonDiagnostic::error(
        "E-DHN-LEX-002",
        "関数呼び出しが閉じられていません。",
        relative_span(base_span, source, start_byte, source.len()),
    ))
}

fn parse_function_content(content: &str, span: Span) -> Result<FunctionCall, DaihonDiagnostic> {
    let args = split_function_args(content);
    let Some((name, rest)) = args.split_first() else {
        return Err(DaihonDiagnostic::error(
            "E-DHN-SYN-050",
            "関数名が空です。",
            span,
        ));
    };
    let mut positional = Vec::new();
    let mut named = BTreeMap::new();
    for arg in rest {
        if let Some(eq_index) = find_assignment_index(arg) {
            let key = normalize_syntax(arg[..eq_index].trim());
            let value = arg[eq_index + 1..].trim();
            named.insert(key, parse_func_arg(value, span)?);
        } else {
            positional.push(parse_func_arg(arg.trim(), span)?);
        }
    }
    Ok(FunctionCall {
        name: Spanned::new(normalize_syntax(name.trim()), span),
        positional,
        named,
        span,
    })
}

fn parse_func_arg(text: &str, span: Span) -> Result<FuncArg, DaihonDiagnostic> {
    let trimmed = text.trim();
    if let Some(inner) = strip_wrapped_paren(trimmed) {
        return parse_arith_expr(inner.trim(), span).map(FuncArg::Expr);
    }
    if trimmed.starts_with('「') {
        let (dialogue, _) = parse_dialogue_at(trimmed, 0, span)?;
        let text = dialogue
            .parts
            .into_iter()
            .filter_map(|part| match part {
                DialoguePart::Text(text) => Some(text.value),
                DialoguePart::Embed(_) => None,
            })
            .collect::<String>();
        return Ok(FuncArg::Expr(Expr::Value(Spanned::new(
            DaihonValue::String(text),
            span,
        ))));
    }
    if let Some(value) = parse_literal(trimmed, span) {
        return Ok(FuncArg::Expr(Expr::Value(Spanned::new(value, span))));
    }
    if trimmed.starts_with('＜') {
        let (function, _) = parse_function_call_at(trimmed, 0, span)?;
        return Ok(FuncArg::Expr(Expr::FunctionCall(function)));
    }
    Ok(FuncArg::BareWord(Spanned::new(
        normalize_syntax(trimmed),
        span,
    )))
}

fn parse_condition_expr(text: &str, span: Span) -> Result<Expr, DaihonDiagnostic> {
    let tokens = expr_tokens(text, span)?;
    let mut parser = ExprStream { tokens, index: 0 };
    let expr = parser.parse_or()?;
    if parser.index < parser.tokens.len() {
        return Err(DaihonDiagnostic::error(
            "E-DHN-SYN-060",
            "条件式の末尾に解釈できない要素があります。",
            parser.tokens[parser.index].span,
        ));
    }
    Ok(expr)
}

fn parse_arith_expr(text: &str, span: Span) -> Result<Expr, DaihonDiagnostic> {
    let tokens = expr_tokens(text, span)?;
    let mut parser = ExprStream { tokens, index: 0 };
    let expr = parser.parse_add_sub()?;
    if parser.index < parser.tokens.len() {
        return Err(DaihonDiagnostic::error(
            "E-DHN-SYN-061",
            "式の末尾に解釈できない要素があります。",
            parser.tokens[parser.index].span,
        ));
    }
    Ok(expr)
}

struct ExprStream {
    tokens: Vec<ExprToken>,
    index: usize,
}

impl ExprStream {
    fn parse_or(&mut self) -> Result<Expr, DaihonDiagnostic> {
        let mut expr = self.parse_and()?;
        while self.consume_op("または").is_some() {
            let right = self.parse_and()?;
            let span = expr.span().join(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Or,
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr, DaihonDiagnostic> {
        let mut expr = self.parse_condition_primary()?;
        while self.consume_op("かつ").is_some() {
            let right = self.parse_condition_primary()?;
            let span = expr.span().join(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::And,
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_condition_primary(&mut self) -> Result<Expr, DaihonDiagnostic> {
        let mut expr = self.parse_condition_primary_base()?;
        while let Some(token) = self.consume_op("でない") {
            let span = expr.span().join(token.span);
            expr = Expr::Not {
                expr: Box::new(expr),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_condition_primary_base(&mut self) -> Result<Expr, DaihonDiagnostic> {
        if let Some(token) = self.peek().cloned() {
            if let ExprTokenKind::Time(start) = token.kind {
                if self
                    .peek_n(1)
                    .map(|t| t.kind == ExprTokenKind::Op("~".to_owned()))
                    .unwrap_or(false)
                {
                    self.index += 2;
                    let end = match self.peek().cloned() {
                        Some(ExprToken {
                            kind: ExprTokenKind::Time(time),
                            ..
                        }) => {
                            self.index += 1;
                            Some(time)
                        }
                        _ => None,
                    };
                    let end_span = end.map(|t| t.span).unwrap_or(token.span);
                    return Ok(Expr::TimeRange {
                        start: Some(start),
                        end,
                        span: token.span.join(end_span),
                    });
                }
            }
            if token.kind == ExprTokenKind::Op("~".to_owned()) {
                self.index += 1;
                if let Some(ExprToken {
                    kind: ExprTokenKind::Time(end),
                    span,
                }) = self.peek().cloned()
                {
                    self.index += 1;
                    return Ok(Expr::TimeRange {
                        start: None,
                        end: Some(end),
                        span,
                    });
                }
            }
        }

        let left = self.parse_add_sub()?;
        if let Some(op) = self.consume_comparison_op() {
            let right = self.parse_add_sub()?;
            let span = left.span().join(right.span());
            return Ok(Expr::Comparison {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            });
        }
        if self.consume_op("~").is_some() {
            let end = if self.peek().is_some() && !self.peek_is_logical() {
                Some(Box::new(self.parse_add_sub()?))
            } else {
                None
            };
            let end_span = end.as_ref().map(|expr| expr.span()).unwrap_or(left.span());
            return Ok(Expr::Range {
                left: Box::new(left),
                start: None,
                end,
                span: end_span,
            });
        }
        if let Some(value_token) = self.peek().cloned() {
            if is_postfix_value_start(&value_token) {
                let save = self.index;
                if let Ok(value) = self.parse_add_sub() {
                    if self.consume_op("~").is_some() {
                        let end = if self.peek().is_some() && !self.peek_is_logical() {
                            Some(Box::new(self.parse_add_sub()?))
                        } else {
                            None
                        };
                        let span = left.span().join(
                            end.as_ref()
                                .map(|expr| expr.span())
                                .unwrap_or_else(|| value.span()),
                        );
                        return Ok(Expr::Range {
                            left: Box::new(left),
                            start: Some(Box::new(value)),
                            end,
                            span,
                        });
                    }
                    if let Some(op) = self.consume_postfix_op() {
                        let span = left.span().join(value.span());
                        return Ok(Expr::PostfixComparison {
                            left: Box::new(left),
                            value: Box::new(value),
                            op,
                            span,
                        });
                    }
                    if let Some(op) = self.consume_string_match_op() {
                        let span = left.span().join(value.span());
                        return Ok(Expr::StringMatch {
                            left: Box::new(left),
                            value: Box::new(value),
                            op,
                            span,
                        });
                    }
                }
                self.index = save;
            }
        }
        Ok(Expr::Truthy {
            span: left.span(),
            expr: Box::new(left),
        })
    }

    fn parse_add_sub(&mut self) -> Result<Expr, DaihonDiagnostic> {
        let mut expr = self.parse_mul_div()?;
        loop {
            let op = if self.consume_op("+").is_some() {
                Some(BinaryOp::Add)
            } else if self.consume_op("-").is_some() {
                Some(BinaryOp::Subtract)
            } else {
                None
            };
            let Some(op) = op else {
                break;
            };
            let right = self.parse_mul_div()?;
            let span = expr.span().join(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_mul_div(&mut self) -> Result<Expr, DaihonDiagnostic> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = if self.consume_op("*").is_some() {
                Some(BinaryOp::Multiply)
            } else if self.consume_op("/").is_some() {
                Some(BinaryOp::Divide)
            } else if self.consume_op("%").is_some() {
                Some(BinaryOp::Modulo)
            } else {
                None
            };
            let Some(op) = op else {
                break;
            };
            let right = self.parse_unary()?;
            let span = expr.span().join(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, DaihonDiagnostic> {
        if let Some(token) = self.consume_op("+") {
            let expr = self.parse_unary()?;
            let span = token.span.join(expr.span());
            return Ok(Expr::Unary {
                op: UnaryOp::Plus,
                expr: Box::new(expr),
                span,
            });
        }
        if let Some(token) = self.consume_op("-") {
            let expr = self.parse_unary()?;
            let span = token.span.join(expr.span());
            return Ok(Expr::Unary {
                op: UnaryOp::Minus,
                expr: Box::new(expr),
                span,
            });
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr, DaihonDiagnostic> {
        let Some(token) = self.next().cloned() else {
            return Err(DaihonDiagnostic::error(
                "E-DHN-SYN-062",
                "式が途中で終わっています。",
                Span::empty(),
            ));
        };
        match token.kind {
            ExprTokenKind::Number(value) => parse_number(&value)
                .map(|number| Expr::Value(Spanned::new(DaihonValue::Number(number), token.span)))
                .ok_or_else(|| {
                    DaihonDiagnostic::error(
                        "E-DHN-SYN-063",
                        "数値を読み取れませんでした。",
                        token.span,
                    )
                }),
            ExprTokenKind::String(value) => Ok(Expr::Value(Spanned::new(
                DaihonValue::String(value),
                token.span,
            ))),
            ExprTokenKind::Bool(value) => Ok(Expr::Value(Spanned::new(
                DaihonValue::Boolean(value),
                token.span,
            ))),
            ExprTokenKind::Ident(name) => Ok(Expr::Variable(parse_variable_ref(&name, token.span))),
            ExprTokenKind::Function(function) => Ok(Expr::FunctionCall(function)),
            ExprTokenKind::LParen => {
                let expr = self.parse_or()?;
                if self
                    .consume_kind(|kind| matches!(kind, ExprTokenKind::RParen))
                    .is_none()
                {
                    return Err(DaihonDiagnostic::error(
                        "E-DHN-SYN-064",
                        "閉じ括弧がありません。",
                        token.span,
                    ));
                }
                Ok(expr)
            }
            _ => Err(DaihonDiagnostic::error(
                "E-DHN-SYN-065",
                "ここには値または変数が必要です。",
                token.span,
            )),
        }
    }

    fn consume_comparison_op(&mut self) -> Option<ComparisonOp> {
        let token = self.peek()?.clone();
        let op = match &token.kind {
            ExprTokenKind::Op(op) if op == "=" || op == "==" => ComparisonOp::Eq,
            ExprTokenKind::Op(op) if op == "!=" => ComparisonOp::Ne,
            ExprTokenKind::Op(op) if op == "<" => ComparisonOp::Lt,
            ExprTokenKind::Op(op) if op == "<=" => ComparisonOp::Lte,
            ExprTokenKind::Op(op) if op == ">" => ComparisonOp::Gt,
            ExprTokenKind::Op(op) if op == ">=" => ComparisonOp::Gte,
            _ => return None,
        };
        self.index += 1;
        Some(op)
    }

    fn consume_postfix_op(&mut self) -> Option<ComparisonOp> {
        let token = self.peek()?.clone();
        let op = match &token.kind {
            ExprTokenKind::Op(op) if op == "未満" => ComparisonOp::Lt,
            ExprTokenKind::Op(op) if op == "以下" => ComparisonOp::Lte,
            ExprTokenKind::Op(op) if op == "以上" => ComparisonOp::Gte,
            ExprTokenKind::Op(op) if op == "超える" => ComparisonOp::Gt,
            _ => return None,
        };
        self.index += 1;
        Some(op)
    }

    fn consume_string_match_op(&mut self) -> Option<StringMatchOp> {
        let token = self.peek()?.clone();
        let op = match &token.kind {
            ExprTokenKind::Op(op) if op == "を含む" => StringMatchOp::Contains,
            ExprTokenKind::Op(op) if op == "で始まる" => StringMatchOp::StartsWith,
            ExprTokenKind::Op(op) if op == "で終わる" => StringMatchOp::EndsWith,
            _ => return None,
        };
        self.index += 1;
        Some(op)
    }

    fn consume_op(&mut self, op: &str) -> Option<ExprToken> {
        self.consume_kind(|kind| matches!(kind, ExprTokenKind::Op(value) if value == op))
    }

    fn consume_kind(
        &mut self,
        predicate: impl FnOnce(&ExprTokenKind) -> bool,
    ) -> Option<ExprToken> {
        let token = self.peek()?.clone();
        if predicate(&token.kind) {
            self.index += 1;
            Some(token)
        } else {
            None
        }
    }

    fn peek(&self) -> Option<&ExprToken> {
        self.tokens.get(self.index)
    }

    fn peek_n(&self, offset: usize) -> Option<&ExprToken> {
        self.tokens.get(self.index + offset)
    }

    fn next(&mut self) -> Option<&ExprToken> {
        let token = self.tokens.get(self.index);
        if token.is_some() {
            self.index += 1;
        }
        token
    }

    fn peek_is_logical(&self) -> bool {
        matches!(self.peek().map(|t| &t.kind), Some(ExprTokenKind::Op(op)) if op == "かつ" || op == "または")
    }
}

fn expr_tokens(text: &str, base_span: Span) -> Result<Vec<ExprToken>, DaihonDiagnostic> {
    let mut tokens = Vec::new();
    let mut byte = 0usize;
    while byte < text.len() {
        let ch = text[byte..].chars().next().unwrap();
        if ch.is_whitespace() || ch == '　' {
            byte += ch.len_utf8();
            continue;
        }
        let span_from = |start: usize, end: usize| relative_span(base_span, text, start, end);
        if ch == '「' {
            let (dialogue, next) = parse_dialogue_at(text, byte, base_span)?;
            let value = dialogue
                .parts
                .into_iter()
                .filter_map(|part| match part {
                    DialoguePart::Text(text) => Some(text.value),
                    DialoguePart::Embed(_) => None,
                })
                .collect::<String>();
            tokens.push(ExprToken {
                kind: ExprTokenKind::String(value),
                span: span_from(byte, next),
            });
            byte = next;
            continue;
        }
        if ch == '＜' {
            let (function, next) = parse_function_call_at(text, byte, base_span)?;
            tokens.push(ExprToken {
                kind: ExprTokenKind::Function(function),
                span: span_from(byte, next),
            });
            byte = next;
            continue;
        }
        if ch == '(' || ch == '（' {
            tokens.push(ExprToken {
                kind: ExprTokenKind::LParen,
                span: span_from(byte, byte + ch.len_utf8()),
            });
            byte += ch.len_utf8();
            continue;
        }
        if ch == ')' || ch == '）' {
            tokens.push(ExprToken {
                kind: ExprTokenKind::RParen,
                span: span_from(byte, byte + ch.len_utf8()),
            });
            byte += ch.len_utf8();
            continue;
        }
        let normalized_ch = normalize_char(ch);
        if "+-*/%~=<>".contains(normalized_ch) || normalized_ch == '!' {
            let start = byte;
            byte += ch.len_utf8();
            if byte < text.len() {
                let next = text[byte..].chars().next().unwrap();
                let next_norm = normalize_char(next);
                if matches!(
                    (normalized_ch, next_norm),
                    ('=', '=') | ('!', '=') | ('<', '=') | ('>', '=')
                ) {
                    byte += next.len_utf8();
                }
            }
            let op = normalize_syntax(&text[start..byte]);
            tokens.push(ExprToken {
                kind: ExprTokenKind::Op(op),
                span: span_from(start, byte),
            });
            continue;
        }
        if is_digit(ch) {
            let start = byte;
            if text[start..].starts_with("0x")
                || text[start..].starts_with("0o")
                || text[start..].starts_with("0b")
            {
                byte += 2;
                while byte < text.len() {
                    let c = text[byte..].chars().next().unwrap();
                    if c.is_ascii_alphanumeric() {
                        byte += c.len_utf8();
                    } else {
                        break;
                    }
                }
            } else {
                let mut saw_colon = false;
                let mut saw_dot = false;
                while byte < text.len() {
                    let c = text[byte..].chars().next().unwrap();
                    if is_digit(c) {
                        byte += c.len_utf8();
                    } else if matches!(c, ':' | '：') && !saw_colon {
                        saw_colon = true;
                        byte += c.len_utf8();
                    } else if matches!(c, '.' | '．') && !saw_dot && !saw_colon {
                        saw_dot = true;
                        byte += c.len_utf8();
                    } else {
                        break;
                    }
                }
                if !saw_colon {
                    for word in [
                        "未満",
                        "以下",
                        "以上",
                        "超える",
                        "でない",
                        "を含む",
                        "で始まる",
                        "で終わる",
                    ] {
                        if normalize_syntax(&text[byte..]).starts_with(word) {
                            break;
                        }
                    }
                }
            }
            let normalized = normalize_syntax(&text[start..byte]);
            if normalized.contains(':') {
                let span = span_from(start, byte);
                tokens.push(ExprToken {
                    kind: ExprTokenKind::Time(parse_time(&normalized, span)?),
                    span,
                });
            } else {
                tokens.push(ExprToken {
                    kind: ExprTokenKind::Number(normalized),
                    span: span_from(start, byte),
                });
            }
            continue;
        }
        if is_ident_start(ch) {
            let start = byte;
            byte += ch.len_utf8();
            while byte < text.len() {
                let c = text[byte..].chars().next().unwrap();
                if is_ident_continue(c) || matches!(c, '#' | '＃') {
                    byte += c.len_utf8();
                } else {
                    break;
                }
            }
            let ident = normalize_syntax(&text[start..byte]);
            let kind = match ident.as_str() {
                "はい" => ExprTokenKind::Bool(true),
                "いいえ" => ExprTokenKind::Bool(false),
                "かつ" | "または" | "未満" | "以下" | "以上" | "超える" | "でない" | "を含む"
                | "で始まる" | "で終わる" => ExprTokenKind::Op(ident),
                _ => ExprTokenKind::Ident(ident),
            };
            tokens.push(ExprToken {
                kind,
                span: span_from(start, byte),
            });
            continue;
        }
        return Err(DaihonDiagnostic::error(
            "E-DHN-SYN-066",
            format!("式の中で使用できない文字「{ch}」があります。"),
            span_from(byte, byte + ch.len_utf8()),
        ));
    }
    Ok(tokens)
}

fn logical_lines(source: &str) -> Vec<LogicalLine> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut line = 1usize;
    let mut column = 1usize;
    let mut current_line = 1usize;
    let mut current_column = 1usize;
    let mut byte = 0usize;
    let mut dialogue = false;
    let mut function_depth = 0usize;
    let mut in_function_string = false;
    let mut line_text_end = None;
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let mut index = 0usize;
    while let Some((pos, ch)) = chars.get(index).copied() {
        byte = pos;
        if !dialogue
            && !in_function_string
            && function_depth == 0
            && (source[pos..].starts_with("$$") || source[pos..].starts_with("＄＄"))
        {
            line_text_end.get_or_insert(pos);
            while let Some((_, c)) = chars.get(index).copied() {
                if c == '\n' || c == '\r' {
                    break;
                }
                index += 1;
            }
            continue;
        }
        if dialogue {
            if function_depth > 0 {
                if in_function_string {
                    if ch == '」' && chars.get(index + 1).map(|(_, c)| *c) == Some('」') {
                        index += 2;
                        current_column += 2;
                        continue;
                    }
                    if ch == '」' {
                        in_function_string = false;
                    }
                } else {
                    match ch {
                        '「' => in_function_string = true,
                        '＜' => function_depth += 1,
                        '＞' => function_depth = function_depth.saturating_sub(1),
                        _ => {}
                    }
                }
            } else {
                if ch == '」' && chars.get(index + 1).map(|(_, c)| *c) == Some('」') {
                    index += 2;
                    current_column += 2;
                    continue;
                }
                if ch == '「' && chars.get(index + 1).map(|(_, c)| *c) == Some('「') {
                    index += 2;
                    current_column += 2;
                    continue;
                }
                if ch == '＜' && chars.get(index + 1).map(|(_, c)| *c) != Some('＜') {
                    function_depth += 1;
                } else if ch == '」' {
                    dialogue = false;
                }
            }
        } else if in_function_string {
            if ch == '」' && chars.get(index + 1).map(|(_, c)| *c) != Some('」') {
                in_function_string = false;
            }
        } else if function_depth > 0 {
            match ch {
                '「' => in_function_string = true,
                '＜' => function_depth += 1,
                '＞' => function_depth = function_depth.saturating_sub(1),
                _ => {}
            }
        } else {
            match ch {
                '「' => dialogue = true,
                '＜' => function_depth += 1,
                _ => {}
            }
        }

        if (ch == '\n' || ch == '\r') && !dialogue && !in_function_string && function_depth == 0 {
            let end = line_text_end.unwrap_or(pos);
            let text = source[start..end].to_owned();
            lines.push(LogicalLine {
                text,
                span: Span::new(start, end, line, column),
            });
            if ch == '\r' && chars.get(index + 1).map(|(_, c)| *c) == Some('\n') {
                index += 1;
            }
            index += 1;
            start = chars
                .get(index)
                .map(|(next, _)| *next)
                .unwrap_or(source.len());
            line += 1;
            current_line = line;
            current_column = 1;
            column = 1;
            line_text_end = None;
            continue;
        }
        if ch == '\n' || ch == '\r' {
            current_line += 1;
            current_column = 1;
        } else {
            current_column += 1;
        }
        index += 1;
    }
    if start <= source.len() {
        let end = line_text_end.unwrap_or(source.len());
        let text = source[start..end].to_owned();
        if !text.is_empty() {
            lines.push(LogicalLine {
                text,
                span: Span::new(start, end, line, column),
            });
        }
    }
    let _ = (byte, current_line, current_column);
    lines
}

fn split_condition_prefix(text: &str, span: Span) -> Option<(Expr, &str)> {
    let trimmed = text.trim_start();
    let after_marker = trimmed.strip_prefix('※')?;
    split_paren_expr(after_marker, span)
}

fn split_paren_expr(text: &str, span: Span) -> Option<(Expr, &str)> {
    let trimmed = text.trim_start();
    let open = trimmed.chars().next()?;
    if open != '（' && open != '(' {
        return None;
    }
    let open_len = open.len_utf8();
    let mut depth = 0i32;
    let mut byte = 0usize;
    for ch in trimmed.chars() {
        let norm = normalize_char(ch);
        if norm == '(' {
            depth += 1;
        } else if norm == ')' {
            depth -= 1;
            if depth == 0 {
                let inner = &trimmed[open_len..byte];
                let rest = &trimmed[byte + ch.len_utf8()..];
                let expr = parse_condition_expr(inner, span).ok()?;
                return Some((expr, rest));
            }
        }
        byte += ch.len_utf8();
    }
    None
}

fn trim_condition_wrapper(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(inner) = strip_wrapped_paren(trimmed) {
        inner
    } else {
        trimmed
    }
}

fn trim_nara(text: &str) -> &str {
    let normalized = normalize_syntax(text);
    if normalized.starts_with("なら") {
        text.trim_start_matches("なら").trim_start()
    } else {
        text
    }
}

fn strip_wrapped_paren(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let first = trimmed.chars().next()?;
    let last = trimmed.chars().next_back()?;
    if normalize_char(first) == '(' && normalize_char(last) == ')' {
        Some(&trimmed[first.len_utf8()..trimmed.len() - last.len_utf8()])
    } else {
        None
    }
}

fn split_function_args(content: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut start = 0usize;
    let mut byte = 0usize;
    let mut paren = 0i32;
    let mut function = 0i32;
    let mut string = false;
    while byte < content.len() {
        let ch = content[byte..].chars().next().unwrap();
        if string {
            if ch == '」' {
                string = false;
            }
            byte += ch.len_utf8();
            continue;
        }
        match ch {
            '「' => string = true,
            '（' | '(' => paren += 1,
            '）' | ')' => paren -= 1,
            '＜' => function += 1,
            '＞' => function -= 1,
            ' ' | '\t' | '　' if paren == 0 && function == 0 => {
                let part = content[start..byte].trim();
                if !part.is_empty() {
                    args.push(part.to_owned());
                }
                byte += ch.len_utf8();
                start = byte;
                continue;
            }
            _ => {}
        }
        byte += ch.len_utf8();
    }
    let part = content[start..].trim();
    if !part.is_empty() {
        args.push(part.to_owned());
    }
    args
}

pub(crate) fn parse_variable_ref(text: &str, span: Span) -> VariableRef {
    let normalized = normalize_syntax(text.trim());
    let spanned = |value: &str| Spanned::new(value.to_owned(), span);
    let parts = normalized.split('#').collect::<Vec<_>>();
    match parts.as_slice() {
        [name] if name.starts_with('_') => VariableRef::Temporary {
            name: spanned(name),
        },
        [name] => VariableRef::EventLocal {
            name: spanned(name),
        },
        ["全体", name] => VariableRef::Global {
            name: spanned(name),
        },
        ["入力", name] => VariableRef::Input {
            name: spanned(name),
        },
        ["住人", actor, name] => VariableRef::Resident {
            actor: spanned(actor),
            name: spanned(name),
        },
        ["関係", subject, object, name] => VariableRef::Relation {
            subject: spanned(subject),
            object: spanned(object),
            name: spanned(name),
        },
        [scope, rest @ ..] => VariableRef::Unsupported {
            scope: spanned(scope),
            parts: rest.iter().map(|part| spanned(part)).collect(),
        },
        [] => VariableRef::EventLocal { name: spanned("") },
    }
}

fn parse_literal(text: &str, span: Span) -> Option<DaihonValue> {
    let normalized = normalize_syntax(text);
    match normalized.as_str() {
        "はい" => Some(DaihonValue::Boolean(true)),
        "いいえ" => Some(DaihonValue::Boolean(false)),
        _ => parse_number(&normalized)
            .map(DaihonValue::Number)
            .or_else(|| {
                if text.starts_with('「') {
                    Some(DaihonValue::String(
                        text.trim_start_matches('「')
                            .trim_end_matches('」')
                            .to_owned(),
                    ))
                } else {
                    let _ = span;
                    None
                }
            }),
    }
}

fn parse_number(text: &str) -> Option<DaihonNumber> {
    let normalized = normalize_syntax(text);
    if let Some(hex) = normalized.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok().map(DaihonNumber::Integer);
    }
    if let Some(oct) = normalized.strip_prefix("0o") {
        return i64::from_str_radix(oct, 8).ok().map(DaihonNumber::Integer);
    }
    if let Some(bin) = normalized.strip_prefix("0b") {
        return i64::from_str_radix(bin, 2).ok().map(DaihonNumber::Integer);
    }
    if normalized.contains('.') {
        normalized.parse::<f64>().ok().map(DaihonNumber::Float)
    } else {
        normalized.parse::<i64>().ok().map(DaihonNumber::Integer)
    }
}

fn parse_time(text: &str, span: Span) -> Result<TimeOfDay, DaihonDiagnostic> {
    let normalized = normalize_syntax(text);
    let mut parts = normalized.split(':');
    let hour = parts.next().and_then(|value| value.parse::<u8>().ok());
    let minute = parts.next().and_then(|value| value.parse::<u8>().ok());
    match (hour, minute) {
        (Some(hour), Some(minute)) if hour <= 23 && minute <= 59 => {
            Ok(TimeOfDay { hour, minute, span })
        }
        _ => Err(DaihonDiagnostic::error(
            "E-DHN-SYN-067",
            "時刻は 0:00 から 23:59 の範囲で指定してください。",
            span,
        )),
    }
}

fn parse_duration(text: &str) -> Option<Duration> {
    let normalized = normalize_syntax(text.trim());
    let digits = normalized
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    let value = digits.parse::<u64>().ok()?;
    let unit = normalized[digits.len()..].trim();
    match unit {
        "秒" | "s" => Some(Duration::from_secs(value)),
        "分" | "m" => Some(Duration::from_secs(value * 60)),
        "時間" | "h" => Some(Duration::from_secs(value * 60 * 60)),
        "日" | "d" => Some(Duration::from_secs(value * 60 * 60 * 24)),
        _ => None,
    }
}

fn find_assignment_index(text: &str) -> Option<usize> {
    let mut byte = 0usize;
    let mut paren = 0i32;
    let mut function = 0i32;
    let mut dialogue = false;
    while byte < text.len() {
        let ch = text[byte..].chars().next().unwrap();
        if dialogue {
            if ch == '」' {
                dialogue = false;
            }
            byte += ch.len_utf8();
            continue;
        }
        match ch {
            '「' => dialogue = true,
            '＜' => function += 1,
            '＞' => function -= 1,
            '(' | '（' => paren += 1,
            ')' | '）' => paren -= 1,
            '=' | '＝' if paren == 0 && function == 0 => {
                let prev = text[..byte].chars().last();
                let next = text[byte + ch.len_utf8()..].chars().next();
                if prev == Some('!') || prev == Some('<') || prev == Some('>') || next == Some('=')
                {
                    byte += ch.len_utf8();
                    continue;
                }
                return Some(byte);
            }
            _ => {}
        }
        byte += ch.len_utf8();
    }
    None
}

fn split_metadata_line(normalized_line: &str) -> Option<(String, &str)> {
    let (key, rest) = normalized_line.split_once(':')?;
    Some((key.trim().to_owned(), rest))
}

fn is_known_metadata_key(key: &str) -> bool {
    matches!(
        key,
        "合図" | "条件" | "優先度" | "重み" | "クールダウン" | "話者" | "前提条件" | "初期値"
    )
}

fn suggest_metadata_key(key: &str) -> Option<String> {
    const KEYS: &[&str] = &["合図", "条件", "優先度", "重み", "クールダウン", "話者"];
    KEYS.iter()
        .find(|candidate| metadata_key_maybe_typo(key, candidate))
        .map(|candidate| (*candidate).to_owned())
}

fn metadata_key_maybe_typo(value: &str, candidate: &str) -> bool {
    value.chars().next() == candidate.chars().next()
        && char_levenshtein(value, candidate) <= if candidate.chars().count() <= 3 { 1 } else { 3 }
}

fn char_levenshtein(left: &str, right: &str) -> usize {
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut current = vec![left_index + 1];
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let cost = usize::from(left_char != *right_char);
            current.push(
                (previous[right_index + 1] + 1)
                    .min(current[right_index] + 1)
                    .min(previous[right_index] + cost),
            );
        }
        previous = current;
    }
    previous[right_chars.len()]
}

fn normalize_line_head(text: &str) -> String {
    normalize_syntax(text.trim())
}

fn is_section_start(text: &str) -> bool {
    let normalized = normalize_line_head(text);
    normalized.starts_with("前提条件:") || normalized.starts_with("初期値:")
}

fn is_scene_header(text: &str) -> bool {
    normalize_line_head(text).starts_with("###")
}

fn split_speaker_prefix(text: &str) -> Option<(&str, &str)> {
    let normalized = normalize_syntax(text);
    let colon = normalized.find(':')?;
    let left = text.get(..colon)?;
    let right = text.get(colon + 1..)?;
    let key = left.trim();
    if key.is_empty()
        || matches!(
            key,
            "合図" | "条件" | "優先度" | "重み" | "クールダウン" | "話者" | "前提条件" | "初期値"
        )
    {
        return None;
    }
    Some((left, right))
}

fn span_for_substr(line: &LogicalLine, needle: &str) -> Span {
    if needle.is_empty() {
        return line.span;
    }
    let offset = line.text.find(needle).unwrap_or(0);
    relative_span(line.span, &line.text, offset, offset + needle.len())
}

fn relative_span(base: Span, source: &str, start: usize, end: usize) -> Span {
    let column_offset = source
        .get(..start)
        .map(|text| text.chars().count())
        .unwrap_or(start);
    Span::new(
        base.start + start,
        base.start + end,
        base.line,
        base.column + column_offset,
    )
}

fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Display(display) => display.span,
        Stmt::SpeakerDisplay { speaker, display } => speaker.span.join(display.span),
        Stmt::Assignment(assignment) => assignment.span,
        Stmt::Jump(jump) => jump.span(),
        Stmt::Conditional(block) => block.span,
    }
}

fn is_postfix_value_start(token: &ExprToken) -> bool {
    matches!(
        token.kind,
        ExprTokenKind::Number(_)
            | ExprTokenKind::String(_)
            | ExprTokenKind::Bool(_)
            | ExprTokenKind::Ident(_)
            | ExprTokenKind::Function(_)
            | ExprTokenKind::LParen
    )
}

fn is_digit(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, '０'..='９')
}

fn is_ident_start(ch: char) -> bool {
    ch == '_'
        || ch == '＿'
        || ch.is_ascii_alphabetic()
        || matches!(
            ch,
            'ぁ'..='ん' | 'ァ'..='ン' | '一'..='龯' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ' | 'ー'
        )
}

fn is_ident_continue(ch: char) -> bool {
    is_ident_start(ch) || is_digit(ch) || matches!(ch, '‥' | '…' | '.' | '#' | 'ー')
}
