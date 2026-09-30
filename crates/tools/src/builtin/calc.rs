//! Safe arithmetic evaluation tool.
//!
//! Hand-written recursive-descent parser: supports `+ - * / % ^`, parentheses,
//! unary minus, and the constants `pi` / `e`. No `eval`, no external deps, no
//! code execution — division by zero and malformed input return clean errors.

use crate::schema::{ToolResult, ToolSchema};
use crate::{RiskLevel, Tool, ToolContext};
use async_trait::async_trait;
use serde_json::json;

/// Evaluate a mathematical expression and return the numeric result.
pub struct CalculateTool;

#[async_trait]
impl Tool for CalculateTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "calculate".into(),
            description: "计算数学表达式。支持 + - * / % ^（幂）、括号、负号，以及常量 pi 和 e。例如：(1+2)*3^2".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "expression": {"type": "string", "description": "要计算的数学表达式"}
                },
                "required": ["expression"]
            }),
        }
    }

    async fn execute(&self, params: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let expr = match params.get("expression").and_then(|v| v.as_str()) {
            Some(e) if !e.trim().is_empty() => e.trim(),
            _ => return ToolResult::err("缺少有效的 expression 参数"),
        };

        match evaluate(expr) {
            Ok(value) => {
                // Print integers without a trailing ".0" for readability.
                let formatted = if value.fract() == 0.0 && value.abs() < 1e15 {
                    format!("{}", value as i64)
                } else {
                    format!("{value}")
                };
                ToolResult::ok(format!("{expr} = {formatted}"))
            }
            Err(e) => ToolResult::err(format!("计算失败: {e}")),
        }
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }
}

/// Evaluate `expr` to an `f64`.
fn evaluate(expr: &str) -> Result<f64, String> {
    let mut parser = Parser::new(expr);
    let value = parser.parse_expr()?;
    parser.skip_ws();
    if !parser.is_eof() {
        return Err(format!("表达式在 \"{}\" 处有多余内容", parser.remaining()));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            input: text.as_bytes(),
            pos: 0,
        }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn remaining(&self) -> &str {
        std::str::from_utf8(&self.input[self.pos..]).unwrap_or("")
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.bump();
        }
    }

    /// expr := term (('+' | '-') term)*
    fn parse_expr(&mut self) -> Result<f64, String> {
        let mut lhs = self.parse_term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'+') => {
                    self.bump();
                    lhs += self.parse_term()?;
                }
                Some(b'-') => {
                    self.bump();
                    lhs -= self.parse_term()?;
                }
                _ => return Ok(lhs),
            }
        }
    }

    /// term := factor (('*' | '/' | '%') factor)*
    fn parse_term(&mut self) -> Result<f64, String> {
        let mut lhs = self.parse_factor()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'*') => {
                    self.bump();
                    lhs *= self.parse_factor()?;
                }
                Some(b'/') => {
                    self.bump();
                    let rhs = self.parse_factor()?;
                    if rhs == 0.0 {
                        return Err("除数为零".into());
                    }
                    lhs /= rhs;
                }
                Some(b'%') => {
                    self.bump();
                    let rhs = self.parse_factor()?;
                    if rhs == 0.0 {
                        return Err("取余运算的除数为零".into());
                    }
                    lhs %= rhs;
                }
                _ => return Ok(lhs),
            }
        }
    }

    /// factor := unary ('^' factor)?   — exponent is right-associative
    fn parse_factor(&mut self) -> Result<f64, String> {
        let base = self.parse_unary()?;
        self.skip_ws();
        if self.peek() == Some(b'^') {
            self.bump();
            let exponent = self.parse_factor()?;
            return Ok(base.powf(exponent));
        }
        Ok(base)
    }

    /// unary := '-' unary | primary
    fn parse_unary(&mut self) -> Result<f64, String> {
        self.skip_ws();
        if self.peek() == Some(b'-') {
            self.bump();
            return Ok(-self.parse_unary()?);
        }
        if self.peek() == Some(b'+') {
            self.bump();
            return self.parse_unary();
        }
        self.parse_primary()
    }

    /// primary := number | ident | '(' expr ')'
    fn parse_primary(&mut self) -> Result<f64, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'(') => {
                self.bump();
                let value = self.parse_expr()?;
                self.skip_ws();
                if self.peek() != Some(b')') {
                    return Err("缺少右括号".into());
                }
                self.bump();
                Ok(value)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.parse_number(),
            Some(c) if c.is_ascii_alphabetic() => {
                let start = self.pos;
                while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
                    self.bump();
                }
                match std::str::from_utf8(&self.input[start..self.pos]).unwrap_or("") {
                    "pi" => Ok(std::f64::consts::PI),
                    "e" => Ok(std::f64::consts::E),
                    name => Err(format!("未知常量或函数: {name}")),
                }
            }
            Some(c) => Err(format!("无法解析的字符: {}", c as char)),
            None => Err("表达式意外结束".into()),
        }
    }

    fn parse_number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        let mut seen_dot = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.bump();
            } else if c == b'.' && !seen_dot {
                seen_dot = true;
                self.bump();
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .ok_or_else(|| "数字解析失败".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(expr: &str) -> f64 {
        evaluate(expr).unwrap()
    }

    #[test]
    fn basic_arithmetic() {
        assert_eq!(eval("1+2"), 3.0);
        assert_eq!(eval("2*3+4"), 10.0);
        assert_eq!(eval("2*(3+4)"), 14.0);
        assert_eq!(eval("10/4"), 2.5);
        assert_eq!(eval("10%3"), 1.0);
    }

    #[test]
    fn exponent_is_right_associative() {
        assert_eq!(eval("2^3^2"), 512.0); // 2^(3^2), not (2^3)^2
        assert_eq!(eval("2^10"), 1024.0);
    }

    #[test]
    fn unary_minus() {
        assert_eq!(eval("-3+5"), 2.0);
        assert_eq!(eval("-(2+3)"), -5.0);
        assert_eq!(eval("2*-3"), -6.0);
    }

    #[test]
    fn constants() {
        assert!((eval("pi") - std::f64::consts::PI).abs() < 1e-12);
        assert!((eval("e") - std::f64::consts::E).abs() < 1e-12);
        assert_eq!(eval("pi*0"), 0.0);
    }

    #[test]
    fn whitespace_tolerance() {
        assert_eq!(eval("  1 + 2 * ( 3 + 4 ) "), 15.0);
    }

    #[test]
    fn errors_are_clean() {
        assert!(evaluate("1/0").unwrap_err().contains("除数为零"));
        assert!(evaluate("(1+2").unwrap_err().contains("右括号"));
        assert!(evaluate("1+").is_err());
        assert!(evaluate("foo").unwrap_err().contains("未知常量"));
        assert!(evaluate("1 2").is_err());
        assert!(evaluate("").is_err());
    }

    #[tokio::test]
    async fn tool_formats_integers_without_decimal() {
        let tool = CalculateTool;
        let ctx = ToolContext::auto_confirm(".");
        let result = tool.execute(json!({"expression": "6*7"}), &ctx).await;
        assert!(result.success);
        assert!(result.output.contains("42"));
        assert!(!result.output.contains("42.0"));
    }
}
