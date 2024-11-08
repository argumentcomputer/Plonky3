use crate::constraint::{ChipMetadata, Expression, Node};
use p3_field::{ExtensionField, Field};
use std::marker::PhantomData;
use thiserror::Error;

use crate::constraint::node::{NodeError, NodesInfo};
use crate::constraint::zerofier::ZerofierExpression;

pub struct RawChipMetadata<F: Field> {
    num_local_variables: Vec<usize>,
    trace_widths: Vec<usize>,
    zerofiers: Vec<ZerofierExpression<F>>,
    periodic: Vec<Vec<F>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

#[derive(Error, Debug)]
pub enum ChipError {
    #[error("node error")]
    NodeError(#[from] NodeError),
    #[error("periodic[{0}]: column length is not a power of 2")]
    Periodic(usize),
    #[error("constraint[{0}]: no/invalid zerofier or invalid node reference")]
    Constraint(usize),
}

impl<F: Field, EF: ExtensionField<F>> TryFrom<RawChipMetadata<F>> for ChipMetadata<F, EF> {
    type Error = ChipError;

    fn try_from(value: RawChipMetadata<F>) -> Result<Self, Self::Error> {
        let RawChipMetadata {
            num_local_variables,
            trace_widths,
            zerofiers,
            periodic,
            nodes,
            constraints,
        } = value;
        let nodes_info = NodesInfo::<F, EF>::new(&nodes)?;

        // Ensure the nodes only reference local variables with `chip_id = 0` and that these
        // are a subset of the predefined number of variables
        nodes_info.validate_local_variables(&num_local_variables)?;

        // Ensure nodes reference trace elements in the correct range
        let trace_window_dimensions = nodes_info.get_dimension(&trace_widths)?;

        // Ensure correct access to periodic columns and their sizes are powers of two
        nodes_info.validate_periodic(periodic.len())?;

        // Ensure periodic columns are power of 2 and multiples of each other
        for (col_idx, col) in periodic.iter().enumerate() {
            if !col.len().is_power_of_two() {
                return Err(ChipError::Periodic(col_idx));
            }
        }

        // Ensure each constraint has a zerofier and that it references valid nodes and zerofiers.
        for (constraint_idx, constraint) in constraints.iter().enumerate() {
            if constraint.node_id >= nodes.len()
                || constraint
                    .zerofier_id
                    .is_some_and(|id| id >= zerofiers.len())
            {
                return Err(ChipError::Constraint(constraint_idx));
            }
        }

        let degrees = nodes_info.get_degrees();

        Ok(Self {
            num_local_variables,
            trace_window_dimensions,
            periodic,
            zerofiers,
            nodes,
            constraints,
            degrees,
            _marker: PhantomData,
        })
    }
}
