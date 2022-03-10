use crate::sys;

/// A unary tensor core operation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    Sqrt,
    Not,
}

impl From<UnaryOp> for sys::cudnnOpTensorOp_t {
    fn from(op: UnaryOp) -> Self {
        match op {
            UnaryOp::Sqrt => Self::CUDNN_OP_TENSOR_SQRT,
            UnaryOp::Not => Self::CUDNN_OP_TENSOR_NOT,
        }
    }
}

/// A binary tensor core operation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinaryOp {
    Add,
    Mul,
    Min,
    Max,
}

impl From<BinaryOp> for sys::cudnnOpTensorOp_t {
    fn from(op: BinaryOp) -> Self {
        match op {
            BinaryOp::Add => Self::CUDNN_OP_TENSOR_ADD,
            BinaryOp::Mul => Self::CUDNN_OP_TENSOR_MUL,
            BinaryOp::Min => Self::CUDNN_OP_TENSOR_MIN,
            BinaryOp::Max => Self::CUDNN_OP_TENSOR_MAX,
        }
    }
}

/// A tensor core operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TensorOp {
    UnaryOp(UnaryOp),
    BinaryOp(BinaryOp),
}

impl From<UnaryOp> for TensorOp {
    fn from(op: UnaryOp) -> Self {
        Self::UnaryOp(op)
    }
}

impl From<BinaryOp> for TensorOp {
    fn from(op: BinaryOp) -> Self {
        Self::BinaryOp(op)
    }
}

impl From<TensorOp> for sys::cudnnOpTensorOp_t {
    fn from(op: TensorOp) -> Self {
        match op {
            TensorOp::BinaryOp(op) => op.into(),
            TensorOp::UnaryOp(op) => op.into(),
        }
    }
}
