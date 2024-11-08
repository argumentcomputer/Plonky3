use crate::constraint::chip_metadata::{ChipError, RawChipMetadata};
use crate::constraint::node::{NodeError, NodesInfo};
use crate::constraint::{ChipMetadata, Expression, MachineMetadata, Node};
use p3_field::{ExtensionField, Field};
use thiserror::Error;

pub struct RawMachineMetadata<F: Field> {
    num_global_variables: Vec<usize>,
    chips: Vec<RawChipMetadata<F>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

#[derive(Error, Debug)]
pub enum MachineError {
    #[error("chip[`{0}`] error")]
    Chip(usize, ChipError),
    #[error("node error")]
    Nodes(#[from] NodeError),
    #[error("constraint[`{0}`]: contains zerofier or invalid node reference")]
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

        let machine_nodes_info = NodesInfo::<F, EF>::new(&nodes)?;

        // No periodic columns
        machine_nodes_info.validate_periodic(0)?;

        // No trace
        machine_nodes_info.get_dimension(&[])?;

        // Check correct global variables
        machine_nodes_info.validate_global_variables(&num_global_variables)?;

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
            .collect::<Result<Vec<ChipMetadata<F, EF>>, MachineError>>()?;

        // Local variable access specific chips
        let shared_variables: Vec<_> = chips.iter().map(|chip| &chip.num_local_variables).collect();
        machine_nodes_info.validate_shared_variables(&shared_variables)?;

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
