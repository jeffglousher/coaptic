//! Block / Q-Block transfer sidecar for a body slot.
//!
//! Each occupied Incoming / Outgoing Body Slot holds one contiguous body and
//! the Block or Q-Block state for that body (`design.md`). Individual CoAP
//! messages stay in ordinary datagram slots. Incoming Q-Block1 / Q-Block2
//! use a fixed `MAX_PAYLOADS` window (RFC 9177 §7.2 default 10). BERT,
//! Q-Block transmit, and random-access recovery are out of scope. See
//! `knowledge/rfcs/rfc7959.txt` and `knowledge/rfcs/rfc9177.txt`.

use super::endpoint::Endpoint;
use super::slot::SlotId;
use crate::error::BlockTransferError;
use crate::message::{BlockValue, Token};

/// Lookup identity for one classic block-wise body.
///
/// RFC 7959 Block1 / Block2 ride the RFC 7252 request/response match: Token
/// plus the remote [`Endpoint`]. This is not a Dedup key (Message ID) and not
/// a seventh core area; it is sidecar on the body slot.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BlockKey {
    token: Token,
    endpoint: Endpoint,
}

impl BlockKey {
    /// Identity for one Token at `endpoint`.
    #[must_use]
    pub const fn new(token: Token, endpoint: Endpoint) -> Self {
        Self { token, endpoint }
    }

    /// Token of the block-wise exchange.
    #[must_use]
    pub const fn token(self) -> Token {
        self.token
    }

    /// Remote endpoint of the block-wise exchange.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.endpoint
    }
}

/// Which body-pool direction this sidecar describes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum BlockRole {
    /// Incoming Block1: client → server request body.
    IncomingBlock1,
    /// Incoming Block2: server → client response body.
    IncomingBlock2,
    /// Incoming Q-Block1: client → server request body (RFC 9177 window).
    IncomingQBlock1,
    /// Incoming Q-Block2: server → client response body (RFC 9177 window).
    IncomingQBlock2,
    /// Outgoing Block1: client → server request body.
    OutgoingBlock1,
    /// Outgoing Block2: server → client response body.
    OutgoingBlock2,
}

impl BlockRole {
    /// Incoming Block1 / Block2 / Q-Block1 / Q-Block2 (RX body pool).
    #[must_use]
    pub const fn is_incoming(self) -> bool {
        matches!(
            self,
            Self::IncomingBlock1
                | Self::IncomingBlock2
                | Self::IncomingQBlock1
                | Self::IncomingQBlock2
        )
    }

    /// Outgoing Block1 or outgoing Block2 (TX body pool).
    #[must_use]
    pub const fn is_outgoing(self) -> bool {
        matches!(self, Self::OutgoingBlock1 | Self::OutgoingBlock2)
    }

    /// Incoming Q-Block1 or incoming Q-Block2.
    #[must_use]
    pub const fn is_q_block(self) -> bool {
        matches!(self, Self::IncomingQBlock1 | Self::IncomingQBlock2)
    }
}

/// Result of writing one incoming Block1 / Block2 range into a body slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockProgress {
    id: SlotId,
    filled: usize,
    complete: bool,
}

impl BlockProgress {
    pub(crate) const fn new(id: SlotId, filled: usize, complete: bool) -> Self {
        Self {
            id,
            filled,
            complete,
        }
    }

    /// Body slot that holds this transfer.
    #[must_use]
    pub const fn id(self) -> SlotId {
        self.id
    }

    /// Bytes assembled so far (complete-body length when [`Self::complete`]).
    #[must_use]
    pub const fn filled(self) -> usize {
        self.filled
    }

    /// Whether M=0 was accepted and the length is consistent.
    #[must_use]
    pub const fn complete(self) -> bool {
        self.complete
    }
}

/// One issued outgoing Block1 / Block2 range. Bytes live in the body slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutgoingBlock {
    id: SlotId,
    block: BlockValue,
    offset: usize,
    len: usize,
    complete: bool,
}

impl OutgoingBlock {
    pub(crate) const fn new(
        id: SlotId,
        block: BlockValue,
        offset: usize,
        len: usize,
        complete: bool,
    ) -> Self {
        Self {
            id,
            block,
            offset,
            len,
            complete,
        }
    }

    /// Body slot that holds the complete body.
    #[must_use]
    pub const fn id(self) -> SlotId {
        self.id
    }

    /// NUM / M / SZX for this issued block.
    #[must_use]
    pub const fn block(self) -> BlockValue {
        self.block
    }

    /// Byte offset of this block in the complete body.
    #[must_use]
    pub const fn offset(self) -> usize {
        self.offset
    }

    /// Payload length of this block.
    #[must_use]
    pub const fn len(self) -> usize {
        self.len
    }

    /// Whether this issued block has no payload bytes.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Whether this was the last block (M=0).
    #[must_use]
    pub const fn complete(self) -> bool {
        self.complete
    }
}

/// Current Q-Block `MAX_PAYLOADS_SET` (RFC 9177 §2 / §7.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QWindow {
    base: u32,
    mask: u16,
    final_num: Option<u32>,
    final_payload_len: u16,
}

/// Block transfer sidecar stored next to one body-slot buffer.
///
/// Classic Block is in-order: the next NUM must be [`Self::next_num`]. Incoming
/// Q-Block tracks a [`Self::MAX_PAYLOADS`]-wide bitmap on the current window
/// and allows out-of-order NUMs inside that window. See `design.md`,
/// `knowledge/rfcs/rfc7959.txt`, and `knowledge/rfcs/rfc9177.txt`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockTransfer {
    key: BlockKey,
    role: BlockRole,
    num: u32,
    szx: u8,
    more: bool,
    next_num: u32,
    filled: usize,
    complete: bool,
    expected_len: Option<u32>,
    q: Option<QWindow>,
}

impl BlockTransfer {
    /// RFC 9177 §7.2 default `MAX_PAYLOADS`; this crate's receive window size.
    pub const MAX_PAYLOADS: u8 = 10;

    /// Incoming sidecar after the first accepted block.
    ///
    /// `role` must be [`BlockRole::IncomingBlock1`] or
    /// [`BlockRole::IncomingBlock2`]. Classic Block starts at NUM 0.
    pub fn incoming(
        key: BlockKey,
        role: BlockRole,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
        expected_len: Option<u32>,
    ) -> Result<Self, BlockTransferError> {
        if !role.is_incoming() || role.is_q_block() {
            return Err(BlockTransferError::IdentityMismatch);
        }
        if block.num() != 0 {
            return Err(BlockTransferError::Gap);
        }
        let mut transfer = Self {
            key,
            role,
            num: 0,
            szx: block.szx(),
            more: block.more(),
            next_num: 0,
            filled: 0,
            complete: false,
            expected_len,
            q: None,
        };
        transfer.accept_incoming(block, payload_len, capacity)?;
        Ok(transfer)
    }

    /// Incoming Block1 sidecar after the first accepted block.
    pub fn incoming_block1(
        key: BlockKey,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
        expected_len: Option<u32>,
    ) -> Result<Self, BlockTransferError> {
        Self::incoming(
            key,
            BlockRole::IncomingBlock1,
            block,
            payload_len,
            capacity,
            expected_len,
        )
    }

    /// Incoming Block2 sidecar after the first accepted block.
    pub fn incoming_block2(
        key: BlockKey,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
        expected_len: Option<u32>,
    ) -> Result<Self, BlockTransferError> {
        Self::incoming(
            key,
            BlockRole::IncomingBlock2,
            block,
            payload_len,
            capacity,
            expected_len,
        )
    }

    /// Incoming Q-Block sidecar after the first accepted block.
    ///
    /// `role` must be [`BlockRole::IncomingQBlock1`] or
    /// [`BlockRole::IncomingQBlock2`]. The first datagram may be any NUM in
    /// window 0 (`0..MAX_PAYLOADS`). See `knowledge/rfcs/rfc9177.txt`.
    pub fn incoming_q(
        key: BlockKey,
        role: BlockRole,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
        expected_len: Option<u32>,
    ) -> Result<Self, BlockTransferError> {
        if !role.is_q_block() {
            return Err(BlockTransferError::IdentityMismatch);
        }
        let mut transfer = Self {
            key,
            role,
            num: 0,
            szx: block.szx(),
            more: block.more(),
            next_num: 0,
            filled: 0,
            complete: false,
            expected_len,
            q: Some(QWindow {
                base: 0,
                mask: 0,
                final_num: None,
                final_payload_len: 0,
            }),
        };
        transfer.accept_q_incoming(block, payload_len, capacity)?;
        Ok(transfer)
    }

    /// Incoming Q-Block1 sidecar after the first accepted block.
    pub fn incoming_q_block1(
        key: BlockKey,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
        expected_len: Option<u32>,
    ) -> Result<Self, BlockTransferError> {
        Self::incoming_q(
            key,
            BlockRole::IncomingQBlock1,
            block,
            payload_len,
            capacity,
            expected_len,
        )
    }

    /// Incoming Q-Block2 sidecar after the first accepted block.
    pub fn incoming_q_block2(
        key: BlockKey,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
        expected_len: Option<u32>,
    ) -> Result<Self, BlockTransferError> {
        Self::incoming_q(
            key,
            BlockRole::IncomingQBlock2,
            block,
            payload_len,
            capacity,
            expected_len,
        )
    }

    /// Outgoing sidecar for a complete body already in the slot.
    ///
    /// `role` must be [`BlockRole::OutgoingBlock1`] or
    /// [`BlockRole::OutgoingBlock2`].
    pub fn outgoing(
        key: BlockKey,
        role: BlockRole,
        body_len: usize,
        szx: u8,
        capacity: usize,
    ) -> Result<Self, BlockTransferError> {
        if !role.is_outgoing() {
            return Err(BlockTransferError::IdentityMismatch);
        }
        let _ = BlockValue::size_from_szx(szx)?;
        if body_len > capacity {
            return Err(BlockTransferError::Overflow);
        }
        if let Ok(len) = u32::try_from(body_len) {
            Ok(Self {
                key,
                role,
                num: 0,
                szx,
                more: false,
                next_num: 0,
                filled: body_len,
                complete: false,
                expected_len: Some(len),
                q: None,
            })
        } else {
            Err(BlockTransferError::Overflow)
        }
    }

    /// Outgoing Block1 sidecar for a complete body already in the slot.
    pub fn outgoing_block1(
        key: BlockKey,
        body_len: usize,
        szx: u8,
        capacity: usize,
    ) -> Result<Self, BlockTransferError> {
        Self::outgoing(key, BlockRole::OutgoingBlock1, body_len, szx, capacity)
    }

    /// Outgoing Block2 sidecar for a complete body already in the slot.
    pub fn outgoing_block2(
        key: BlockKey,
        body_len: usize,
        szx: u8,
        capacity: usize,
    ) -> Result<Self, BlockTransferError> {
        Self::outgoing(key, BlockRole::OutgoingBlock2, body_len, szx, capacity)
    }

    /// Lookup identity.
    #[must_use]
    pub const fn key(self) -> BlockKey {
        self.key
    }

    /// Token of the transfer.
    #[must_use]
    pub const fn token(self) -> Token {
        self.key.token()
    }

    /// Remote endpoint of the transfer.
    #[must_use]
    pub const fn endpoint(self) -> Endpoint {
        self.key.endpoint()
    }

    /// Incoming or outgoing Block / Q-Block role.
    #[must_use]
    pub const fn role(self) -> BlockRole {
        self.role
    }

    /// Whether this sidecar is an incoming Q-Block window.
    #[must_use]
    pub const fn is_q_block(self) -> bool {
        self.role.is_q_block()
    }

    /// Last accepted (incoming) or last issued (outgoing) NUM.
    #[must_use]
    pub const fn num(self) -> u32 {
        self.num
    }

    /// SZX locked for this transfer.
    #[must_use]
    pub const fn szx(self) -> u8 {
        self.szx
    }

    /// M flag of the last accepted or issued block.
    #[must_use]
    pub const fn more(self) -> bool {
        self.more
    }

    /// Next NUM classic Block will accept or issue.
    #[must_use]
    pub const fn next_num(self) -> u32 {
        self.next_num
    }

    /// Bytes assembled (incoming) or complete-body length (outgoing).
    #[must_use]
    pub const fn filled(self) -> usize {
        self.filled
    }

    /// Whether the transfer has accepted M=0 or issued the last outgoing block.
    #[must_use]
    pub const fn is_complete(self) -> bool {
        self.complete
    }

    /// Size1 / Size2 hint when known.
    #[must_use]
    pub const fn expected_len(self) -> Option<u32> {
        self.expected_len
    }

    /// First NUM of the current Q-Block window (`MAX_PAYLOADS_SET`).
    #[must_use]
    pub const fn window_base(self) -> u32 {
        match self.q {
            Some(q) => q.base,
            None => 0,
        }
    }

    /// Bitmask of received NUMs in the current window (`bit i` is `base + i`).
    #[must_use]
    pub const fn window_mask(self) -> u16 {
        match self.q {
            Some(q) => q.mask,
            None => 0,
        }
    }

    /// Accept one in-order incoming block. Returns the write offset.
    pub fn accept_incoming(
        &mut self,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
    ) -> Result<usize, BlockTransferError> {
        if !self.role.is_incoming() || self.role.is_q_block() {
            return Err(BlockTransferError::IdentityMismatch);
        }
        if self.complete {
            return Err(BlockTransferError::AlreadyComplete);
        }
        if block.szx() != self.szx {
            return Err(BlockTransferError::SzxMismatch);
        }
        if block.num() < self.next_num {
            return Err(BlockTransferError::Overlap);
        }
        if block.num() > self.next_num {
            return Err(BlockTransferError::Gap);
        }

        let size = usize::from(block.size());
        if block.more() {
            if payload_len != size {
                return Err(BlockTransferError::PayloadLength);
            }
        } else if payload_len > size {
            return Err(BlockTransferError::PayloadLength);
        }

        let offset = block_offset(block.num(), size)?;
        let end = offset
            .checked_add(payload_len)
            .ok_or(BlockTransferError::Overflow)?;
        if end > capacity {
            return Err(BlockTransferError::Overflow);
        }
        if let Some(expected) = self.expected_len {
            let expected = usize::try_from(expected).map_err(|_| BlockTransferError::Overflow)?;
            if end > expected || (!block.more() && end != expected) {
                return Err(BlockTransferError::LengthInconsistent);
            }
        }

        self.num = block.num();
        self.more = block.more();
        self.filled = end;
        self.next_num = block
            .num()
            .checked_add(1)
            .ok_or(BlockTransferError::Overflow)?;
        self.complete = !block.more();
        Ok(offset)
    }

    /// Accept one incoming Q-Block in the current window. Returns the write offset.
    ///
    /// Out-of-order NUMs inside `[window_base, window_base + MAX_PAYLOADS)` are
    /// stored. Duplicates and NUMs outside that window are rejected. The window
    /// advances when every slot in the current `MAX_PAYLOADS_SET` is filled.
    /// The body completes when the M=0 block is stored and the prefix has no
    /// gaps. See `knowledge/rfcs/rfc9177.txt`.
    pub fn accept_q_incoming(
        &mut self,
        block: BlockValue,
        payload_len: usize,
        capacity: usize,
    ) -> Result<usize, BlockTransferError> {
        if !self.role.is_q_block() {
            return Err(BlockTransferError::IdentityMismatch);
        }
        if self.complete {
            return Err(BlockTransferError::AlreadyComplete);
        }
        if block.szx() != self.szx {
            return Err(BlockTransferError::SzxMismatch);
        }

        let expected_len = self.expected_len;
        let q = self
            .q
            .as_mut()
            .ok_or(BlockTransferError::IdentityMismatch)?;
        if block.num() < q.base {
            return Err(BlockTransferError::Duplicate);
        }
        let idx = block.num() - q.base;
        if idx >= u32::from(Self::MAX_PAYLOADS) {
            return Err(BlockTransferError::OutsideWindow);
        }
        let bit = 1u16 << idx;
        if q.mask & bit != 0 {
            return Err(BlockTransferError::Duplicate);
        }
        if let Some(final_num) = q.final_num {
            if block.num() > final_num {
                return Err(BlockTransferError::LengthInconsistent);
            }
        }
        if !block.more() {
            let after = idx.saturating_add(1);
            if after < u32::from(Self::MAX_PAYLOADS) && (q.mask >> after) != 0 {
                return Err(BlockTransferError::LengthInconsistent);
            }
        }

        let size = usize::from(block.size());
        if block.more() {
            if payload_len != size {
                return Err(BlockTransferError::PayloadLength);
            }
        } else if payload_len > size {
            return Err(BlockTransferError::PayloadLength);
        }
        let final_payload_len =
            u16::try_from(payload_len).map_err(|_| BlockTransferError::Overflow)?;

        let offset = block_offset(block.num(), size)?;
        let end = offset
            .checked_add(payload_len)
            .ok_or(BlockTransferError::Overflow)?;
        if end > capacity {
            return Err(BlockTransferError::Overflow);
        }
        if let Some(expected) = expected_len {
            let expected = usize::try_from(expected).map_err(|_| BlockTransferError::Overflow)?;
            if end > expected || (!block.more() && end != expected) {
                return Err(BlockTransferError::LengthInconsistent);
            }
        }

        q.mask |= bit;
        if !block.more() {
            q.final_num = Some(block.num());
            q.final_payload_len = final_payload_len;
        }

        let window_full = (1u16 << Self::MAX_PAYLOADS) - 1;
        while q.mask == window_full && q.final_num.is_none() {
            q.base = q
                .base
                .checked_add(u32::from(Self::MAX_PAYLOADS))
                .ok_or(BlockTransferError::Overflow)?;
            q.mask = 0;
        }

        let contig = q
            .base
            .checked_add(q.mask.trailing_ones())
            .ok_or(BlockTransferError::Overflow)?;
        if let Some(final_num) = q.final_num {
            if contig > final_num {
                self.filled = block_offset(final_num, size)?
                    .checked_add(usize::from(q.final_payload_len))
                    .ok_or(BlockTransferError::Overflow)?;
                self.complete = true;
                self.next_num = final_num
                    .checked_add(1)
                    .ok_or(BlockTransferError::Overflow)?;
            } else {
                self.filled = block_offset(contig, size)?;
                self.complete = false;
                self.next_num = contig;
            }
        } else {
            self.filled = block_offset(contig, size)?;
            self.complete = false;
            self.next_num = contig;
        }

        self.num = block.num();
        self.more = block.more();
        Ok(offset)
    }

    /// Issue the next in-order outgoing block. Returns `(block, offset, len)`.
    pub fn issue_outgoing(&mut self) -> Result<(BlockValue, usize, usize), BlockTransferError> {
        if !self.role.is_outgoing() {
            return Err(BlockTransferError::IdentityMismatch);
        }
        if self.complete {
            return Err(BlockTransferError::AlreadyComplete);
        }

        let size = usize::from(BlockValue::size_from_szx(self.szx)?);
        let offset = block_offset(self.next_num, size)?;
        if offset > self.filled {
            return Err(BlockTransferError::Gap);
        }
        let remaining = self.filled - offset;
        let len = remaining.min(size);
        let more = remaining > size;
        let block = BlockValue::new(self.next_num, more, self.szx)?;

        self.num = self.next_num;
        self.more = more;
        self.next_num = self
            .next_num
            .checked_add(1)
            .ok_or(BlockTransferError::Overflow)?;
        self.complete = !more;
        Ok((block, offset, len))
    }
}

pub(crate) fn block_offset(num: u32, size: usize) -> Result<usize, BlockTransferError> {
    usize::try_from(num)
        .ok()
        .and_then(|n| n.checked_mul(size))
        .ok_or(BlockTransferError::Overflow)
}

/// Start a classic or Q-Block incoming sidecar from `role`.
pub(crate) fn start_incoming(
    key: BlockKey,
    role: BlockRole,
    block: BlockValue,
    payload_len: usize,
    capacity: usize,
    expected_len: Option<u32>,
) -> Result<BlockTransfer, BlockTransferError> {
    if role.is_q_block() {
        BlockTransfer::incoming_q(key, role, block, payload_len, capacity, expected_len)
    } else {
        BlockTransfer::incoming(key, role, block, payload_len, capacity, expected_len)
    }
}

/// Accept the next classic or Q-Block incoming range from `transfer.role()`.
pub(crate) fn accept_incoming_role(
    transfer: &mut BlockTransfer,
    block: BlockValue,
    payload_len: usize,
    capacity: usize,
) -> Result<usize, BlockTransferError> {
    if transfer.role().is_q_block() {
        transfer.accept_q_incoming(block, payload_len, capacity)
    } else {
        transfer.accept_incoming(block, payload_len, capacity)
    }
}

/// Write `payload` at `offset`. `contiguous` is the visible prefix (Q-Block holes).
///
/// When `contiguous` is `None`, the filled length becomes `offset + payload.len()`
/// (classic in-order).
pub(crate) fn store_incoming(
    buf: &mut [u8],
    filled: &mut usize,
    offset: usize,
    payload: &[u8],
    contiguous: Option<usize>,
) -> Result<(), BlockTransferError> {
    let end = offset
        .checked_add(payload.len())
        .ok_or(BlockTransferError::Overflow)?;
    if end > buf.len() {
        return Err(BlockTransferError::Overflow);
    }
    buf[offset..end].copy_from_slice(payload);
    *filled = contiguous.unwrap_or(end);
    Ok(())
}

/// Byte and transfer access shared by [`super::BodyPool`] and the alloc body pool.
pub(crate) trait BodyOps {
    fn payload(&self, id: SlotId) -> Option<&[u8]>;
    fn transfer(&self, id: SlotId) -> Option<BlockTransfer>;
    fn lookup(&self, key: BlockKey) -> Option<SlotId>;
    fn admit_incoming(
        &mut self,
        key: BlockKey,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<SlotId, BlockTransferError>;
    fn write_incoming(
        &mut self,
        id: SlotId,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
    ) -> Result<BlockProgress, BlockTransferError>;
    fn apply_incoming(
        &mut self,
        key: BlockKey,
        role: BlockRole,
        block: BlockValue,
        payload: &[u8],
        expected_len: Option<u32>,
    ) -> Result<BlockProgress, BlockTransferError>;
    fn start_outgoing(
        &mut self,
        key: BlockKey,
        role: BlockRole,
        body: &[u8],
        szx: u8,
    ) -> Result<SlotId, BlockTransferError>;
    fn next_outgoing(
        &mut self,
        id: SlotId,
        role: BlockRole,
    ) -> Result<OutgoingBlock, BlockTransferError>;
}

/// Copy `payload` into `buf` at `offset` and record the filled length.
pub(crate) fn write_range(
    buf: &mut [u8],
    filled: &mut usize,
    offset: usize,
    payload: &[u8],
) -> Result<(), BlockTransferError> {
    let end = offset
        .checked_add(payload.len())
        .ok_or(BlockTransferError::Overflow)?;
    if end > buf.len() {
        return Err(BlockTransferError::Overflow);
    }
    buf[offset..end].copy_from_slice(payload);
    *filled = end;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BlockKey, BlockRole, BlockTransfer};
    use crate::error::BlockTransferError;
    use crate::message::{BlockValue, Token};
    use crate::storage::Endpoint;

    fn key() -> BlockKey {
        BlockKey::new(
            Token::new(&[0xaa, 0xbb]).expect("token"),
            Endpoint::v4([192, 0, 2, 1], 5683),
        )
    }

    fn szx16(num: u32, more: bool) -> BlockValue {
        BlockValue::from_size(num, more, 16).expect("szx 0")
    }

    #[test]
    fn incoming_single_block_completes() {
        let t =
            BlockTransfer::incoming_block1(key(), szx16(0, false), 8, 4096, None).expect("admit");
        assert_eq!(t.role(), BlockRole::IncomingBlock1);
        assert_eq!(t.num(), 0);
        assert!(!t.more());
        assert_eq!(t.filled(), 8);
        assert!(t.is_complete());
        assert_eq!(t.next_num(), 1);
    }

    #[test]
    fn incoming_multi_block_in_order() {
        let mut t = BlockTransfer::incoming_block1(key(), szx16(0, true), 16, 4096, Some(40))
            .expect("first");
        assert!(!t.is_complete());
        assert_eq!(t.filled(), 16);
        assert_eq!(t.accept_incoming(szx16(1, true), 16, 4096).expect("2"), 16);
        assert_eq!(
            t.accept_incoming(szx16(2, false), 8, 4096).expect("last"),
            32
        );
        assert!(t.is_complete());
        assert_eq!(t.filled(), 40);
    }

    #[test]
    fn incoming_rejects_gap_overlap_szx_overflow() {
        assert_eq!(
            BlockTransfer::incoming_block1(key(), szx16(1, false), 8, 4096, None)
                .expect_err("num 1"),
            BlockTransferError::Gap
        );

        let mut t =
            BlockTransfer::incoming_block1(key(), szx16(0, true), 16, 4096, None).expect("first");
        assert_eq!(
            t.accept_incoming(szx16(2, true), 16, 4096)
                .expect_err("gap"),
            BlockTransferError::Gap
        );
        assert_eq!(
            t.accept_incoming(szx16(0, true), 16, 4096)
                .expect_err("overlap"),
            BlockTransferError::Overlap
        );
        let szx1024 = BlockValue::from_size(1, true, 1024).expect("1024");
        assert_eq!(
            t.accept_incoming(szx1024, 1024, 4096).expect_err("szx"),
            BlockTransferError::SzxMismatch
        );

        let big = BlockValue::from_size(0, true, 1024).expect("1024");
        let mut full = BlockTransfer::incoming_block1(key(), big, 1024, 4096, None).expect("b0");
        for n in 1..4 {
            let b = BlockValue::from_size(n, true, 1024).expect("blk");
            full.accept_incoming(b, 1024, 4096).expect("fit");
        }
        let extra = BlockValue::from_size(4, false, 1024).expect("overflow");
        assert_eq!(
            full.accept_incoming(extra, 1, 4096).expect_err("cap"),
            BlockTransferError::Overflow
        );
    }

    #[test]
    fn incoming_size1_must_match_on_m0() {
        let err = BlockTransfer::incoming_block1(key(), szx16(0, false), 8, 4096, Some(16))
            .expect_err("short");
        assert_eq!(err, BlockTransferError::LengthInconsistent);
    }

    #[test]
    fn outgoing_issues_in_order() {
        let mut t = BlockTransfer::outgoing_block2(key(), 40, 0, 4096).expect("start");
        assert_eq!(t.role(), BlockRole::OutgoingBlock2);
        let (b0, off0, len0) = t.issue_outgoing().expect("b0");
        assert_eq!((b0.num(), b0.more(), off0, len0), (0, true, 0, 16));
        let (b1, off1, len1) = t.issue_outgoing().expect("b1");
        assert_eq!((b1.num(), b1.more(), off1, len1), (1, true, 16, 16));
        let (b2, off2, len2) = t.issue_outgoing().expect("b2");
        assert_eq!((b2.num(), b2.more(), off2, len2), (2, false, 32, 8));
        assert!(t.is_complete());
        assert_eq!(
            t.issue_outgoing().expect_err("done"),
            BlockTransferError::AlreadyComplete
        );
    }

    #[test]
    fn outgoing_rejects_body_past_capacity() {
        assert_eq!(
            BlockTransfer::outgoing_block2(key(), 4097, 6, 4096).expect_err("cap"),
            BlockTransferError::Overflow
        );
    }

    #[test]
    fn incoming_block2_single_and_multi() {
        let t =
            BlockTransfer::incoming_block2(key(), szx16(0, false), 8, 4096, None).expect("admit");
        assert_eq!(t.role(), BlockRole::IncomingBlock2);
        assert!(t.role().is_incoming());
        assert!(!t.role().is_outgoing());
        assert_eq!(t.num(), 0);
        assert!(!t.more());
        assert_eq!(t.filled(), 8);
        assert!(t.is_complete());

        let mut multi = BlockTransfer::incoming_block2(key(), szx16(0, true), 16, 4096, Some(40))
            .expect("first");
        assert_eq!(
            multi.accept_incoming(szx16(1, true), 16, 4096).expect("2"),
            16
        );
        assert_eq!(
            multi
                .accept_incoming(szx16(2, false), 8, 4096)
                .expect("last"),
            32
        );
        assert!(multi.is_complete());
        assert_eq!(multi.filled(), 40);
    }

    #[test]
    fn incoming_block2_rejects_gap_overlap_szx_overflow() {
        assert_eq!(
            BlockTransfer::incoming_block2(key(), szx16(1, false), 8, 4096, None)
                .expect_err("num 1"),
            BlockTransferError::Gap
        );

        let mut t =
            BlockTransfer::incoming_block2(key(), szx16(0, true), 16, 4096, None).expect("first");
        assert_eq!(
            t.accept_incoming(szx16(2, true), 16, 4096)
                .expect_err("gap"),
            BlockTransferError::Gap
        );
        assert_eq!(
            t.accept_incoming(szx16(0, true), 16, 4096)
                .expect_err("overlap"),
            BlockTransferError::Overlap
        );
        let szx1024 = BlockValue::from_size(1, true, 1024).expect("1024");
        assert_eq!(
            t.accept_incoming(szx1024, 1024, 4096).expect_err("szx"),
            BlockTransferError::SzxMismatch
        );

        let big = BlockValue::from_size(0, true, 1024).expect("1024");
        let mut full = BlockTransfer::incoming_block2(key(), big, 1024, 4096, None).expect("b0");
        for n in 1..4 {
            let b = BlockValue::from_size(n, true, 1024).expect("blk");
            full.accept_incoming(b, 1024, 4096).expect("fit");
        }
        let extra = BlockValue::from_size(4, false, 1024).expect("overflow");
        assert_eq!(
            full.accept_incoming(extra, 1, 4096).expect_err("cap"),
            BlockTransferError::Overflow
        );
    }

    #[test]
    fn incoming_size2_must_match_on_m0() {
        let err = BlockTransfer::incoming_block2(key(), szx16(0, false), 8, 4096, Some(16))
            .expect_err("short");
        assert_eq!(err, BlockTransferError::LengthInconsistent);
    }

    #[test]
    fn outgoing_block1_issues_in_order() {
        let mut t = BlockTransfer::outgoing_block1(key(), 40, 0, 4096).expect("start");
        assert_eq!(t.role(), BlockRole::OutgoingBlock1);
        assert!(t.role().is_outgoing());
        assert!(!t.role().is_incoming());
        let (b0, off0, len0) = t.issue_outgoing().expect("b0");
        assert_eq!((b0.num(), b0.more(), off0, len0), (0, true, 0, 16));
        let (b1, off1, len1) = t.issue_outgoing().expect("b1");
        assert_eq!((b1.num(), b1.more(), off1, len1), (1, true, 16, 16));
        let (b2, off2, len2) = t.issue_outgoing().expect("b2");
        assert_eq!((b2.num(), b2.more(), off2, len2), (2, false, 32, 8));
        assert!(t.is_complete());
        assert_eq!(
            t.issue_outgoing().expect_err("done"),
            BlockTransferError::AlreadyComplete
        );
    }

    #[test]
    fn outgoing_block1_rejects_body_past_capacity() {
        assert_eq!(
            BlockTransfer::outgoing_block1(key(), 4097, 6, 4096).expect_err("cap"),
            BlockTransferError::Overflow
        );
    }

    #[test]
    fn constructors_reject_wrong_direction_role() {
        assert_eq!(
            BlockTransfer::incoming(
                key(),
                BlockRole::OutgoingBlock1,
                szx16(0, false),
                8,
                4096,
                None
            )
            .expect_err("out as in"),
            BlockTransferError::IdentityMismatch
        );
        assert_eq!(
            BlockTransfer::outgoing(key(), BlockRole::IncomingBlock2, 8, 0, 4096)
                .expect_err("in as out"),
            BlockTransferError::IdentityMismatch
        );
        assert_eq!(
            BlockTransfer::incoming(
                key(),
                BlockRole::IncomingQBlock1,
                szx16(0, false),
                8,
                4096,
                None
            )
            .expect_err("q as classic"),
            BlockTransferError::IdentityMismatch
        );
        assert_eq!(
            BlockTransfer::incoming_q(
                key(),
                BlockRole::IncomingBlock1,
                szx16(0, false),
                8,
                4096,
                None
            )
            .expect_err("classic as q"),
            BlockTransferError::IdentityMismatch
        );
    }

    #[test]
    fn incoming_cannot_issue_outgoing_and_vice_versa() {
        let mut incoming =
            BlockTransfer::incoming_block2(key(), szx16(0, true), 16, 4096, None).expect("in");
        assert_eq!(
            incoming.issue_outgoing().expect_err("in"),
            BlockTransferError::IdentityMismatch
        );
        let mut outgoing = BlockTransfer::outgoing_block1(key(), 16, 0, 4096).expect("out");
        assert_eq!(
            outgoing
                .accept_incoming(szx16(0, false), 8, 4096)
                .expect_err("out"),
            BlockTransferError::IdentityMismatch
        );
    }

    #[test]
    fn q_block_out_of_order_within_window() {
        let mut t = BlockTransfer::incoming_q_block1(key(), szx16(2, false), 8, 4096, Some(40))
            .expect("num 2 first");
        assert_eq!(t.role(), BlockRole::IncomingQBlock1);
        assert!(t.is_q_block());
        assert_eq!(t.window_base(), 0);
        assert_eq!(t.filled(), 0);
        assert!(!t.is_complete());
        assert_eq!(t.accept_q_incoming(szx16(0, true), 16, 4096).expect("0"), 0);
        assert_eq!(t.filled(), 16);
        assert_eq!(t.next_num(), 1);
        assert_eq!(
            t.accept_q_incoming(szx16(1, true), 16, 4096).expect("1"),
            16
        );
        assert!(t.is_complete());
        assert_eq!(t.filled(), 40);
    }

    #[test]
    fn q_block_duplicate_and_outside_window() {
        let mut t =
            BlockTransfer::incoming_q_block2(key(), szx16(0, true), 16, 4096, None).expect("0");
        assert_eq!(t.role(), BlockRole::IncomingQBlock2);
        assert_eq!(
            t.accept_q_incoming(szx16(0, true), 16, 4096)
                .expect_err("dup"),
            BlockTransferError::Duplicate
        );
        assert_eq!(
            t.accept_q_incoming(
                szx16(u32::from(BlockTransfer::MAX_PAYLOADS), true),
                16,
                4096
            )
            .expect_err("next window"),
            BlockTransferError::OutsideWindow
        );
    }

    #[test]
    fn q_block_advances_window_then_completes() {
        let last = u32::from(BlockTransfer::MAX_PAYLOADS) + 1;
        let expected = (last as usize) * 16 + 8;
        let mut t = BlockTransfer::incoming_q_block1(
            key(),
            szx16(0, true),
            16,
            4096,
            Some(expected as u32),
        )
        .expect("0");
        for n in 1..BlockTransfer::MAX_PAYLOADS {
            t.accept_q_incoming(szx16(u32::from(n), true), 16, 4096)
                .expect("window 0");
        }
        assert_eq!(t.window_base(), u32::from(BlockTransfer::MAX_PAYLOADS));
        assert_eq!(t.window_mask(), 0);
        assert!(!t.is_complete());
        t.accept_q_incoming(szx16(last, false), 8, 4096)
            .expect("final first");
        assert!(!t.is_complete());
        t.accept_q_incoming(
            szx16(u32::from(BlockTransfer::MAX_PAYLOADS), true),
            16,
            4096,
        )
        .expect("gap fill");
        assert!(t.is_complete());
        assert_eq!(t.filled(), expected);
    }
}
