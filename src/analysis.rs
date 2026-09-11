use serde::Serialize;
use tree_sitter::{Node, Parser};

/// A function-like construct found in a TypeScript source file.
#[derive(Debug, Clone, Serialize)]
pub struct Function {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    pub complexity: usize,
}

/// Parse and analyse TypeScript (or TSX when `tsx` is true).
///
/// Complexity is cyclomatic complexity: one for the function itself, plus one
/// for each branch listed in `is_decision`. Nested functions and classes are
/// analysed independently. Decisions in parameter default expressions are
/// counted, since parameters are part of the function AST.
pub fn analyze(source: &str, tsx: bool) -> Result<Vec<Function>, String> {
    let mut parser = Parser::new();
    let language = if tsx {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    };
    parser
        .set_language(&language)
        .map_err(|e| format!("failed to load TypeScript grammar: {e}"))?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "TypeScript parser did not produce a syntax tree".to_owned())?;

    if let Some(node) = first_invalid(tree.root_node()) {
        let point = node.start_position();
        let problem = if node.is_missing() {
            format!("missing {}", node.kind())
        } else {
            "syntax error".to_owned()
        };
        return Err(format!(
            "{problem} at line {}, column {}",
            point.row + 1,
            point.column + 1
        ));
    }

    let mut functions = Vec::new();
    collect_functions(tree.root_node(), source, &mut functions);
    Ok(functions)
}

fn first_invalid(node: Node<'_>) -> Option<Node<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(invalid) = first_invalid(child) {
            return Some(invalid);
        }
    }
    None
}

fn collect_functions(node: Node<'_>, source: &str, output: &mut Vec<Function>) {
    if is_function(node) && has_body(node) {
        let start = node.start_position();
        let end = node.end_position();
        output.push(Function {
            name: function_name(node, source),
            start_line: start.row + 1,
            // tree-sitter's end point is exclusive. A construct ending at the
            // beginning of a line belongs to the preceding line.
            end_line: if end.column == 0 && end.row > start.row {
                end.row
            } else {
                end.row + 1
            },
            complexity: 1 + count_decisions(node, node, source),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(child, source, output);
    }
}

fn is_function(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "method_definition"
    )
}

fn has_body(node: Node<'_>) -> bool {
    node.child_by_field_name("body").is_some()
}

fn count_decisions(node: Node<'_>, root: Node<'_>, source: &str) -> usize {
    if node.id() != root.id() && (is_function(node) || is_class(node)) {
        return 0;
    }

    let mut count = usize::from(is_decision(node, source));
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        count += count_decisions(child, root, source);
    }
    count
}

fn is_class(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "class_declaration" | "abstract_class_declaration" | "class"
    )
}

fn is_decision(node: Node<'_>, source: &str) -> bool {
    match node.kind() {
        "if_statement"
        | "for_statement"
        | "for_in_statement"
        | "while_statement"
        | "do_statement"
        | "catch_clause"
        | "ternary_expression"
        | "conditional_expression"
        | "switch_case"
        | "switch_default" => true,
        "binary_expression" => node
            .child_by_field_name("operator")
            .and_then(|operator| text(operator, source))
            .is_some_and(|operator| matches!(operator, "&&" | "||")),
        _ => false,
    }
}

fn function_name(node: Node<'_>, source: &str) -> String {
    let own_name = node
        .child_by_field_name("name")
        .and_then(|name| text(name, source))
        .map(str::trim)
        .filter(|name| !name.is_empty());

    let inferred = own_name.or_else(|| inferred_binding_name(node, source));
    let name = inferred
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| anonymous_name(node));

    if node.kind() == "method_definition"
        && let Some(class_name) = enclosing_class_name(node, source)
    {
        return format!("{class_name}.{name}");
    }
    name
}

fn inferred_binding_name<'a>(node: Node<'a>, source: &'a str) -> Option<&'a str> {
    let parent = node.parent()?;
    let field = match parent.kind() {
        "variable_declarator" => "name",
        "pair" => "key",
        "public_field_definition" | "field_definition" => "name",
        "assignment_expression" => "left",
        _ => return None,
    };
    parent
        .child_by_field_name(field)
        .and_then(|binding| text(binding, source))
        .map(str::trim)
}

fn enclosing_class_name<'a>(node: Node<'a>, source: &'a str) -> Option<&'a str> {
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if is_class(parent) {
            if let Some(name) = parent.child_by_field_name("name") {
                return text(name, source).map(str::trim);
            }
            // A class expression commonly gets its useful name from a variable.
            return inferred_binding_name(parent, source);
        }
        ancestor = parent.parent();
    }
    None
}

fn anonymous_name(node: Node<'_>) -> String {
    let point = node.start_position();
    format!("<anonymous@{}:{}>", point.row + 1, point.column + 1)
}

fn text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    source.get(node.byte_range())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by_name<'a>(functions: &'a [Function], name: &str) -> &'a Function {
        functions.iter().find(|f| f.name == name).unwrap()
    }

    #[test]
    fn decision_rules_are_independent() {
        for (body, expected) in [
            ("if (a) {} else {}", 2),
            ("for (;;) {}", 2),
            ("for (const x in xs) {}", 2),
            ("for (const x of xs) {}", 2),
            ("while (a) {}", 2),
            ("do {} while (a);", 2),
            ("try {} catch (e) {} finally {}", 2),
            ("return a ? b : c;", 2),
            (
                "switch(a) { case 1: break; case 2: break; default: break; }",
                4,
            ),
            ("return a && b && c;", 3),
            ("return a || b;", 2),
            ("return a ?? b?.c;", 1),
            ("a &&= b; a ||= c; a ??= d;", 1),
            ("return `if && ${a ? b : c}`;", 2),
        ] {
            let source = format!("function f() {{ {body} }}");
            assert_eq!(
                analyze(&source, false).unwrap()[0].complexity,
                expected,
                "{body}"
            );
        }
    }

    #[test]
    fn counts_every_decision_but_nullish_and_optional_chaining() {
        let source = r#"
function all(x: any) {
  if (x) x = x && 1 || 2;
  for (;;) break;
  for (const y in x) break;
  while (x) break;
  do {} while (x);
  try {} catch (e) {}
  x ? 1 : 2;
  switch (x) { case 1: break; default: break; }
  return x ?? x?.value;
}
"#;
        let result = analyze(source, false).unwrap();
        // baseline + if + && + || + 4 loops + catch + ternary + 2 cases
        assert_eq!(by_name(&result, "all").complexity, 12);
    }

    #[test]
    fn extracts_names_and_qualifies_methods_accessors_and_constructor() {
        let source = r#"
const arrow = () => 1;
const expr = function named() {};
const obj = { prop: function () {}, short() {} };
class Box {
  constructor() {}
  get value() { return 1; }
  set value(x: number) {}
  method() {}
  declared(): void;
}
"#;
        let result = analyze(source, false).unwrap();
        let names: Vec<_> = result.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"arrow"));
        assert!(names.contains(&"named"));
        assert!(names.contains(&"prop"));
        assert!(names.contains(&"short"));
        assert!(names.contains(&"Box.constructor"));
        assert!(names.contains(&"Box.value"));
        assert!(names.contains(&"Box.method"));
        assert!(!names.iter().any(|name| name.contains("declared")));
    }

    #[test]
    fn nested_functions_and_classes_are_isolated() {
        let source = r#"
function outer() {
  if (ok) {}
  const inner = () => { if (ok) {} while (ok) {} };
  class Local { method() { if (ok) {} } }
}
"#;
        let result = analyze(source, false).unwrap();
        assert_eq!(by_name(&result, "outer").complexity, 2);
        assert_eq!(by_name(&result, "inner").complexity, 3);
        assert_eq!(by_name(&result, "Local.method").complexity, 2);
    }

    #[test]
    fn abstract_classes_isolate_initializers_and_qualify_methods() {
        let source = r#"
function outer(a: boolean, b: boolean) {
  abstract class Local {
    value = a || b;
    method() { if (a) return b; return false; }
    abstract declared(): void;
  }
}
"#;
        let result = analyze(source, false).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(by_name(&result, "outer").complexity, 1);
        assert_eq!(by_name(&result, "Local.method").complexity, 2);
    }

    #[test]
    fn counts_decisions_in_parameter_defaults() {
        let result = analyze("function f(x = a || b, y = a ? b : c) {}", false).unwrap();
        assert_eq!(by_name(&result, "f").complexity, 3);
    }

    #[test]
    fn comments_and_strings_are_not_code() {
        let result = analyze(
            r#"function clean() { const s = "if (x) while (y) a && b"; /* catch {} */ }"#,
            false,
        )
        .unwrap();
        assert_eq!(result[0].complexity, 1);
    }

    #[test]
    fn supports_tsx_and_rejects_it_in_typescript_mode() {
        let source = "const View = () => <div>{ok ? <b /> : null}</div>;";
        let result = analyze(source, true).unwrap();
        assert_eq!(by_name(&result, "View").complexity, 2);
        assert!(analyze(source, false).is_err());
    }

    #[test]
    fn skips_overloads_but_keeps_implementation() {
        let source = "function f(x: string): string;\nfunction f(x: number): number;\nfunction f(x: any) { return x; }";
        let result = analyze(source, false).unwrap();
        assert_eq!(result.iter().filter(|f| f.name == "f").count(), 1);
    }

    #[test]
    fn rejects_error_and_missing_nodes() {
        assert!(analyze("function broken( {", false).is_err());
        assert!(analyze("const x = ;", false).is_err());
    }

    #[test]
    fn anonymous_name_contains_one_based_location_and_lines_are_inclusive() {
        let result = analyze("const xs = [\n  function () {\n  }\n];", false).unwrap();
        assert_eq!(result[0].name, "<anonymous@2:3>");
        assert_eq!((result[0].start_line, result[0].end_line), (2, 3));
    }
}
