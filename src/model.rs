//! Format-independent API model shared by extraction, rendering, and diff.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageModel {
    pub package: String,
    pub access_level: String,
    pub modules: Vec<ModuleModel>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModuleModel {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<SymbolNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<ExtensionGroup>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionGroup {
    pub extended_module: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<SymbolNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolNode {
    pub decl: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<SymbolNode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_without_doc_fields_still_parses() {
        let yaml = "\
package: P
access_level: public
modules:
- name: M
  symbols:
  - decl: public struct S
    members:
    - decl: 'public var x: Int'
";
        let model: PackageModel = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(model.modules[0].symbols[0].doc, None);
        assert_eq!(model.modules[0].symbols[0].source, None);
    }

    #[test]
    fn yaml_omits_empty_doc_fields() {
        let model = PackageModel {
            package: "P".to_string(),
            access_level: "public".to_string(),
            modules: vec![ModuleModel {
                name: "M".to_string(),
                symbols: vec![SymbolNode {
                    decl: "public struct S".to_string(),
                    doc: None,
                    source: None,
                    members: vec![],
                }],
                extensions: vec![],
            }],
        };
        let yaml = serde_yaml::to_string(&model).unwrap();
        assert!(!yaml.contains("doc:"));
        assert!(!yaml.contains("source:"));
    }
}
