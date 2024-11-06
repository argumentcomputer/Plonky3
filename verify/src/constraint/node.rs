use p3_field::{ExtensionField, Field};
use p3_matrix::Dimensions;
use std::cmp;
use std::marker::PhantomData;

#[derive(Copy, Clone, Debug)]
enum VarScope {
    /// Refer to a shared variable (public value, challenge, ...)
    Global,
    /// Refer to a local variable (permutation hint, ...)
    /// When evaluating a trace, `chip_id` must be 0 to refer to itself
    Local { chip_id: usize },
}

#[derive(Copy, Clone, Debug)]
enum FieldType {
    Base,
    Ext,
}

#[derive(Copy, Clone, Debug)]
pub enum Node<F: Field> {
    Constant(F),
    /// Base or extension field element from a list of local traces
    Trace {
        segment: usize,
        col_offset: usize,
        row_offset: usize,
        field_type: FieldType,
    },
    /// Base or extension field element local from global (e.g. public values or challenges) or
    /// local (permutation argument hints) variable.
    Var {
        scope: VarScope,
        group: usize,
        offset: usize,
        field_type: FieldType,
    },
    /// Base field elements from local periodic columns
    Periodic {
        column: usize,
    },
    Add {
        lhs_id: usize,
        rhs_id: usize,
    },
    Sub {
        lhs_id: usize,
        rhs_id: usize,
    },
    Mul {
        lhs_id: usize,
        rhs_id: usize,
    },
}

pub struct ProcessedNodes<F: Field, EF: ExtensionField<F>> {
    pub nodes: Vec<Node<F>>,
    pub counted_local_variables: Vec<Vec<usize>>,
    pub counted_global_variables: Vec<usize>,
    pub counted_periodic_columns: usize,
    pub trace_window_dimensions: Vec<Dimensions>,
    pub degrees: Vec<usize>,
    _marker: PhantomData<EF>,
}

impl<F: Field, EF: ExtensionField<F>> TryFrom<Vec<Node<F>>> for ProcessedNodes<F, EF> {
    type Error = ();

    fn try_from(nodes: Vec<Node<F>>) -> Result<Self, Self::Error> {
        let mut counted_local_variables = vec![vec![]];
        let mut counted_global_variables = vec![];
        let mut counted_periodic_columns = 0;
        let mut trace_window_dimensions = vec![];
        let mut degrees = Vec::with_capacity(nodes.len());

        for node in &nodes {
            let degree = match node {
                Node::Constant(_) => 0,
                Node::Trace {
                    segment,
                    col_offset,
                    row_offset,
                    field_type,
                } => {
                    let min_segments = segment + 1;
                    trace_window_dimensions.resize(
                        min_segments,
                        Dimensions {
                            width: 0,
                            height: 0,
                        },
                    );

                    let dim = &mut trace_window_dimensions[*segment];

                    let min_width = match field_type {
                        FieldType::Base => col_offset + 1,
                        FieldType::Ext => col_offset + EF::D,
                    };
                    let min_height = row_offset + 1;
                    dim.width = cmp::max(dim.width, min_width);
                    dim.height = cmp::max(dim.height, min_height);

                    1
                }
                Node::Var {
                    scope,
                    group,
                    offset,
                    field_type,
                } => {
                    let variable_group = match scope {
                        VarScope::Global => &mut counted_global_variables,
                        VarScope::Local { chip_id } => {
                            counted_local_variables.resize(chip_id + 1, vec![]);
                            &mut counted_local_variables[*chip_id]
                        }
                    };
                    let min_width = match field_type {
                        FieldType::Base => offset + 1,
                        FieldType::Ext => offset + EF::D,
                    };
                    variable_group[*group] = cmp::max(variable_group[*group], min_width);

                    0
                }
                Node::Periodic { column } => {
                    counted_periodic_columns = cmp::max(counted_periodic_columns, column + 1);
                    1
                }
                Node::Add { lhs_id, rhs_id } | Node::Sub { lhs_id, rhs_id } => {
                    let lhs_degree = degrees.get(*lhs_id).ok_or(())?;
                    let rhs_degree = degrees.get(*rhs_id).ok_or(())?;
                    cmp::max(*lhs_degree, *rhs_degree)
                }
                Node::Mul { lhs_id, rhs_id } => {
                    let lhs_degree = degrees.get(*lhs_id).ok_or(())?;
                    let rhs_degree = degrees.get(*rhs_id).ok_or(())?;
                    lhs_degree + rhs_degree
                }
            };
            degrees.push(degree);
        }

        Ok(Self {
            nodes,
            counted_local_variables,
            counted_global_variables,
            counted_periodic_columns,
            trace_window_dimensions,
            degrees,
            _marker: PhantomData,
        })
    }
}

impl<F: Field> Node<F> {
    /// Given the evaluations of the preceding nodes, evaluates self over the extension field.
    ///
    pub fn eval<EF: ExtensionField<F>>(
        &self,
        prev_evals: &[EF],
        global_variables: &[Vec<F>],
        chip_variables: &[Vec<Vec<F>>],
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
                    bases
                        .iter()
                        .enumerate()
                        .map(|(e_i, base_i)| EF::monomial(e_i) * *base_i)
                        .sum()
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
                    VarScope::Local { chip_id } => &chip_variables[chip_id],
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
