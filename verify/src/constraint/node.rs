use crate::constraint::{unflatten_extension, FieldType, Node, VarScope, VariableGroupInfo};
use core::slice;
use p3_field::{ExtensionField, Field};
use p3_matrix::Dimensions;
use std::cmp;
use std::marker::PhantomData;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NodeError {
    #[error("node[`{0}`]: invalid arithmetic id reference")]
    InvalidReference(usize),
    #[error("node[`{0}`]: invalid variable reference")]
    Variable(usize),
    #[error("node[`{0}`]: invalid trace reference")]
    Trace(usize),
    #[error("node[`{0}`]: invalid periodic reference")]
    Periodic(usize),
}

pub(super) struct NodesInfo<'a, F: Field, EF: ExtensionField<F>> {
    nodes: &'a [Node<F>],
    _marker: PhantomData<EF>,
}

impl<'a, F: Field, EF: ExtensionField<F>> NodesInfo<'a, F, EF> {
    pub fn new(nodes: &'a [Node<F>]) -> Result<Self, NodeError> {
        for (node_id, node) in nodes.iter().enumerate() {
            match node {
                Node::Add { lhs_id, rhs_id }
                | Node::Sub { lhs_id, rhs_id }
                | Node::Mul { lhs_id, rhs_id } => {
                    if *lhs_id >= node_id || *rhs_id >= node_id {
                        return Err(NodeError::InvalidReference(node_id));
                    }
                }
                _ => {}
            }
        }
        Ok(Self::new_unchecked(nodes))
    }

    pub(crate) fn new_unchecked(nodes: &'a [Node<F>]) -> Self {
        Self {
            nodes,
            _marker: PhantomData,
        }
    }

    /// Checks that all `Node::Var` with `VarScope::Local` reference valid entries
    /// in the shared variable list.
    pub fn validate_shared_variables(
        &self,
        num_shared_vars: &[&VariableGroupInfo],
    ) -> Result<(), NodeError> {
        for (node_id, node) in self.nodes.iter().enumerate() {
            // Iterate over all local variables
            if let Node::Var {
                scope: VarScope::Local { chip_id },
                group,
                offset,
                field_type,
            } = node
            {
                // Ensure we reference a valid group of variables for the chip id
                if num_shared_vars
                    .get(*chip_id)
                    .and_then(|variable_groups| variable_groups.get(*group))
                    .is_none_or(|variable_group_size| {
                        Self::element_exceeds_width(*variable_group_size, *offset, *field_type)
                    })
                {
                    return Err(NodeError::Variable(node_id));
                }
            }
        }

        Ok(())
    }

    pub fn validate_local_variables(
        &self,
        num_local_variables: &VariableGroupInfo,
    ) -> Result<(), NodeError> {
        self.validate_shared_variables(slice::from_ref(&num_local_variables))
    }

    pub fn validate_global_variables(
        &self,
        num_variables: &VariableGroupInfo,
    ) -> Result<(), NodeError> {
        for (node_id, node) in self.nodes.iter().enumerate() {
            // Iterate over all local variables //TODO
            if let Node::Var {
                scope: VarScope::Global,
                group,
                offset,
                field_type,
            } = node
            {
                if num_variables.get(*group).is_none_or(|variable_group_size| {
                    Self::element_exceeds_width(*variable_group_size, *offset, *field_type)
                }) {
                    return Err(NodeError::Variable(node_id));
                }
            }
        }

        Ok(())
    }

    pub fn validate_periodic(&self, num_periodic_columns: usize) -> Result<(), NodeError> {
        for (node_id, node) in self.nodes.iter().enumerate() {
            if let Node::Periodic { column } = node {
                if *column >= num_periodic_columns {
                    return Err(NodeError::Periodic(node_id));
                }
            }
        }
        Ok(())
    }

    pub fn get_degrees(&self) -> Vec<usize> {
        let mut degrees = Vec::with_capacity(self.nodes.len());
        for node in self.nodes {
            degrees.push(match node {
                Node::Constant(_) | Node::Var { .. } => 0,
                Node::Trace { .. } | Node::Periodic { .. } => 1,
                Node::Add { lhs_id, rhs_id } | Node::Sub { lhs_id, rhs_id } => {
                    cmp::max(degrees[*lhs_id], degrees[*rhs_id])
                }
                Node::Mul { lhs_id, rhs_id } => degrees[*lhs_id] + degrees[*rhs_id],
            })
        }
        degrees
    }

    pub fn get_dimension(&self, trace_widths: &[usize]) -> Result<Vec<Dimensions>, NodeError> {
        let mut dims: Vec<_> = trace_widths
            .iter()
            .map(|&width| Dimensions { width, height: 0 })
            .collect();
        for (node_id, node) in self.nodes.iter().enumerate() {
            if let Node::Trace {
                segment,
                col_offset,
                row_offset,
                field_type,
            } = node
            {
                if dims.get_mut(*segment).is_none_or(|dim| {
                    // Set height
                    dim.height = cmp::max(dim.height, row_offset + 1);
                    Self::element_exceeds_width(dim.width, *col_offset, *field_type)
                }) {
                    return Err(NodeError::Trace(node_id));
                }
            }
        }
        Ok(dims)
    }

    /// Check if a buffer of length `width` would be able to hold an `field_type` element at
    /// the given `offset`.
    fn element_exceeds_width(width: usize, offset: usize, field_type: FieldType) -> bool {
        let element_width = match field_type {
            FieldType::Base => 1,
            FieldType::Ext => EF::D,
        };
        offset + element_width > width
    }
}

impl<F: Field> Node<F> {
    /// Given the evaluations of the preceding nodes, evaluates self over the extension field.
    /// Assumes all inputs have been validated.
    pub fn eval<EF: ExtensionField<F>>(
        &self,
        prev_evals: &[EF],
        global_variables: &[Vec<F>],
        local_variables: &[Vec<Vec<F>>],
        trace_evals: &[Vec<Vec<EF>>],
        periodic_evals: &[EF],
    ) -> EF {
        match *self {
            Self::Constant(c) => c.into(),
            Self::Trace {
                segment,
                col_offset,
                row_offset,
                field_type,
            } => match field_type {
                FieldType::Base => trace_evals[segment][row_offset][col_offset],
                FieldType::Ext => {
                    let bases = &trace_evals[segment][row_offset][col_offset..col_offset + EF::D];
                    unflatten_extension(bases)
                }
            },
            Self::Var {
                scope,
                group,
                offset,
                field_type,
            } => {
                let variables = match scope {
                    VarScope::Global => global_variables,
                    VarScope::Local { chip_id } => &local_variables[chip_id],
                };
                let data = &variables[group];
                match field_type {
                    FieldType::Base => EF::from_base(data[offset]),
                    FieldType::Ext => EF::from_base_slice(&data[offset..(offset + EF::D)]),
                }
            }
            Self::Periodic { column } => periodic_evals[column],
            Self::Add { lhs_id, rhs_id } => prev_evals[lhs_id] + prev_evals[rhs_id],
            Self::Sub { lhs_id, rhs_id } => prev_evals[lhs_id] - prev_evals[rhs_id],
            Self::Mul { lhs_id, rhs_id } => prev_evals[lhs_id] * prev_evals[rhs_id],
        }
    }
}
