use crate::constraint::node::{Node, ProcessedNodes};
use crate::constraint::zerofier::ZerofierExpression;
use crate::constraint::{ChipMetadata, Expression, MachineMetadata};
use p3_field::{ExtensionField, Field};
use std::iter::zip;
use std::marker::PhantomData;

pub struct RawMachineMetadata<F: Field> {
    num_global_variables: Vec<usize>,
    chips: Vec<RawChipMetadata<F>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

pub struct RawChipMetadata<F: Field> {
    num_local_variables: Vec<usize>,
    trace_widths_base: Vec<usize>,
    zerofiers: Vec<ZerofierExpression<F>>,
    periodic: Vec<Vec<F>>,
    nodes: Vec<Node<F>>,
    constraints: Vec<Expression>,
}

impl<F: Field, EF: ExtensionField<F>> TryFrom<RawMachineMetadata<F>> for MachineMetadata<F, EF> {
    type Error = ();

    fn try_from(value: RawMachineMetadata<F>) -> Result<Self, Self::Error> {
        let RawMachineMetadata {
            num_global_variables,
            chips,
            nodes,
            constraints,
        } = value;
        let chips = chips
            .into_iter()
            .map(ChipMetadata::try_from)
            .collect::<Result<Vec<ChipMetadata<F, EF>>, _>>()?;

        let ProcessedNodes::<F, EF> {
            nodes,
            counted_local_variables,
            counted_global_variables,
            counted_periodic_columns,
            trace_window_dimensions,
            ..
        } = nodes.try_into()?;

        if counted_local_variables.len() >= chips.len() {
            return Err(());
        }

        if counted_periodic_columns != 0 {
            return Err(());
        }
        if !trace_window_dimensions.is_empty() {
            return Err(());
        }

        size_vec_is_subset(&num_global_variables, &counted_global_variables)?;
        for (chip, counted_local_vars) in zip(&chips, counted_local_variables) {
            size_vec_is_subset(&num_global_variables, &chip.num_global_variables)?;
            size_vec_is_subset(&chip.num_local_variables, &counted_local_vars)?;
        }

        for constraint in &constraints {
            if constraint.node_id >= nodes.len() || constraint.zerofier_id.is_none() {
                return Err(());
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

impl<F: Field, EF: ExtensionField<F>> TryFrom<RawChipMetadata<F>> for ChipMetadata<F, EF> {
    type Error = ();

    fn try_from(value: RawChipMetadata<F>) -> Result<Self, Self::Error> {
        let RawChipMetadata {
            num_local_variables,
            trace_widths_base: trace_widths,
            zerofiers,
            periodic,
            nodes,
            constraints,
        } = value;
        let ProcessedNodes::<F, EF> {
            nodes,
            counted_local_variables,
            counted_global_variables,
            counted_periodic_columns,
            trace_window_dimensions,
            degrees,
            ..
        } = nodes.try_into()?;

        if counted_local_variables.len() != 1 {
            return Err(());
        }
        size_vec_is_subset(&num_local_variables, &counted_local_variables[0])?;

        if trace_widths.len() != trace_window_dimensions.len() {
            return Err(());
        }
        if zip(trace_widths, &trace_window_dimensions)
            .any(|(expected_width, dim)| expected_width != dim.width)
        {
            return Err(());
        }

        if periodic.len() != counted_periodic_columns {
            return Err(());
        }
        for periodic_column in &periodic {
            if !periodic_column.len().is_power_of_two() {
                return Err(());
            }
        }

        for constraint in &constraints {
            if constraint.node_id >= nodes.len() {
                return Err(());
            }

            constraint
                .zerofier_id
                .and_then(|zerofier_id| zerofiers.get(zerofier_id))
                .ok_or(())?;
        }

        Ok(Self {
            num_local_variables,
            num_global_variables: counted_global_variables,
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

fn size_vec_is_subset(big: &[usize], small: &[usize]) -> Result<(), ()> {
    if big.len() < small.len() {
        return Err(());
    }

    if zip(big, small).any(|(big_count, small_count)| big_count < small_count) {
        return Err(());
    }

    Ok(())
}
