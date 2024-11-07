use crate::constraint::node::{Node, NodeError, NodesInfo};
use crate::constraint::zerofier::ZerofierExpression;
use crate::constraint::{ChipMetadata, Expression, MachineMetadata};
use p3_field::{ExtensionField, Field};
use std::marker::PhantomData;

pub struct RawMachineMetadata<F: Field> {
    num_global_variables: Vec<usize>,
    chips: Vec<RawChipMetadata<F>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

pub struct RawChipMetadata<F: Field> {
    num_local_variables: Vec<usize>,
    trace_widths: Vec<usize>,
    zerofiers: Vec<ZerofierExpression<F>>,
    periodic: Vec<Vec<F>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

pub enum MachineError {
    Chip(usize, ChipError),
    Nodes(NodeError),
    Constraint(usize),
}

impl<F: Field, EF: ExtensionField<F>> TryFrom<RawMachineMetadata<F>> for MachineMetadata<F, EF> {
    type Error = MachineError;

    fn try_from(value: RawMachineMetadata<F>) -> Result<Self, Self::Error> {
        let RawMachineMetadata {
            num_global_variables,
            chips,
            nodes,
            constraints,
        } = value;

        let machine_nodes_info = NodesInfo::<F, EF>::new(&nodes).map_err(MachineError::Nodes)?;

        // No periodic columns
        machine_nodes_info
            .validate_periodic(0)
            .map_err(MachineError::Nodes)?;

        // No trace
        machine_nodes_info
            .get_dimension(&[])
            .map_err(MachineError::Nodes)?;

        // Check correct global variables
        machine_nodes_info
            .validate_global_variables(&num_global_variables)
            .map_err(MachineError::Nodes)?;

        let chips = chips
            .into_iter()
            .enumerate()
            .map(|(chip_id, chip)| {
                let chip: ChipMetadata<F, EF> = chip
                    .try_into()
                    .map_err(|err| MachineError::Chip(chip_id, err))?;

                // ensure the chip's constraints reference valid global variables
                let chip_nodes_info = chip.node_info();
                chip_nodes_info
                    .validate_global_variables(&num_global_variables)
                    .map_err(|err| MachineError::Chip(chip_id, ChipError::NodeError(err)))?;

                Ok(chip)
            })
            .collect::<Result<Vec<ChipMetadata<F, EF>>, _>>()?;

        // Local variable access specific chips
        let shared_variables: Vec<_> = chips.iter().map(|chip| &chip.num_local_variables).collect();
        machine_nodes_info
            .validate_shared_variables(&shared_variables)
            .map_err(MachineError::Nodes)?;

        // For each constraint, ensure it references a valid node and it contains no zerofier
        for (constraint_idx, constraint) in constraints.iter().enumerate() {
            if constraint.node_id >= nodes.len() || constraint.zerofier_id.is_some() {
                return Err(MachineError::Constraint(constraint_idx));
            }
        }

        Ok(Self {
            num_global_variables,
            chips,
            nodes,
            constraints,
        })
    }
}

pub enum ChipError {
    NodeError(NodeError),
    Periodic(usize),
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
        let nodes_info = NodesInfo::<F, EF>::new(&nodes).map_err(ChipError::NodeError)?;

        // Ensure the nodes only reference local variables with `chip_id = 0` and that these
        // are a subset of the predefined number of variables
        nodes_info
            .validate_local_variables(&num_local_variables)
            .map_err(ChipError::NodeError)?;

        // Ensure nodes reference trace elements in the correct range
        let trace_window_dimensions = nodes_info
            .get_dimension(&trace_widths)
            .map_err(ChipError::NodeError)?;

        // Ensure correct access to periodic columns and their sizes are powers of two
        nodes_info
            .validate_periodic(periodic.len())
            .map_err(ChipError::NodeError)?;

        // Ensure periodic columns are power of 2
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
