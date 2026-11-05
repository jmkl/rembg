//! The registry of node types the editor knows about.
use crate::node::{CompnodeNode, schema_of};
use compnode_core::hash::FxMap;
use compnode_ui::schema::{NodeSchema, Registry};

/// One registered node type.
pub struct NodeType {
    /// The schema the editor draws the node from.
    pub schema: NodeSchema,
    /// Makes a fresh instance of the node.
    make: Box<dyn Fn() -> Box<dyn CompnodeNode> + Send + Sync>,
}

impl NodeType {
    /// A new, empty instance of this node type.
    pub fn create(&self) -> Box<dyn CompnodeNode> {
        (self.make)()
    }
}

/// Every node type the editor knows.
#[derive(Default)]
pub struct NodeTypes {
    types: FxMap<String, NodeType>,
}

impl NodeTypes {
    pub fn new() -> NodeTypes {
        NodeTypes {
            types: FxMap::default(),
        }
    }

    /// Register a node type. The type is asked for its schema through a
    /// default instance, so registering is the only step needed to make a
    /// node appear in the add-node menu.
    pub fn register<T: CompnodeNode + Default>(&mut self) -> &mut Self {
        self.register_with::<T>(T::default)
    }

    /// Register a node type with a custom constructor. The type is asked for
    /// its schema through one instance made by `make`.
    pub fn register_with<T: CompnodeNode>(
        &mut self,
        make: impl Fn() -> T + Send + Sync + 'static,
    ) -> &mut Self {
        let proto = make();
        let schema = schema_of(&proto);
        let make = move || Box::new(make()) as Box<dyn CompnodeNode>;
        self.types.insert(
            schema.id.clone(),
            NodeType {
                schema,
                make: Box::new(make),
            },
        );
        self
    }

    pub fn get(&self, id: &str) -> Option<&NodeType> {
        self.types.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &NodeType> {
        self.types.values()
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Add every registered schema to a document registry.
    pub fn fill_registry(&self, reg: &mut Registry) {
        let mut all: Vec<&NodeSchema> = self.types.values().map(|t| &t.schema).collect();
        all.sort_by(|a, b| a.id.cmp(&b.id));
        for s in all {
            reg.register(s.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compnode_ui::schema::Category;

    #[derive(Default)]
    struct A;
    impl CompnodeNode for A {
        fn id(&self) -> &'static str {
            "t.a"
        }
        fn title(&self) -> &str {
            "A"
        }
        fn category(&self) -> Category {
            Category::Utility
        }
        fn process(
            &mut self,
            _ctx: &mut crate::node::ProcessCtx,
        ) -> Result<Vec<crate::node::Payload>, crate::node::NodeError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn registering_asks_the_node_for_its_schema() {
        let mut t = NodeTypes::new();
        t.register::<A>();
        assert_eq!(t.len(), 1);
        assert!(t.get("t.a").is_some());
        assert_eq!(t.get("t.a").unwrap().schema.title, "A");
    }
}
