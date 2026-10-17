//! Original 1.2 mailbox packets. Inventory and balances remain owned by item tasks.

use crate::model::Item;
use crate::opcodes::{cs, sc};
use crate::session::Event;
use crate::wire::{Reader, Result, WireError, Writer};

pub const MAX_ATTACHMENTS: usize = 10;

pub mod mail_type {
    pub const INVALID_MAIL_TYPE: u8 = 0;
    pub const NORMAL: u8 = 1;
    pub const EXPRESS: u8 = 2;
    pub const REGISTERED: u8 = 3;
    pub const SPAM: u8 = 4;
    pub const ADMIN: u8 = 5;
    pub const BILLING: u8 = 6;
    pub const BALANCE_RECEIPT: u8 = 7;
    pub const TAXRATE_CHANGED: u8 = 8;
    pub const CHARGED: u8 = 9;
    pub const PROMOTION: u8 = 10;
    pub const MIA_SEND: u8 = 11;
    pub const MIA_RECV: u8 = 12;
    pub const AUC_OFF_CANCEL: u8 = 13;
    pub const AUC_OFF_SUCCESS: u8 = 14;
    pub const AUC_OFF_FAIL: u8 = 15;
    pub const AUC_BID_WIN: u8 = 16;
    pub const AUC_BID_FAIL: u8 = 17;
    pub const SYS_EXPRESS: u8 = 18;
    pub const SYS_SELL_BACKPACK: u8 = 19;
    pub const DEMOLISH_WITH_PENALTY: u8 = 20;
    pub const HOUSING_SALE: u8 = 21;
    pub const USER_BILL: u8 = 22;
    pub const NATION_TAX_RATE: u8 = 23;
    pub const NATION_TAX_RECEIPT: u8 = 24;
    pub const NATION_REQUEST_FRIEND: u8 = 25;
    pub const KICKED_FROM_BEGINNER_EXPED: u8 = 26;
    pub const MOBILIZATION_GIVE_ITEM: u8 = 27;
    pub const HERO_CANDIDATE_ALARM: u8 = 28;
    pub const HERO_ELECTION_ITEM: u8 = 29;
    pub const HOUSING_REBUILD: u8 = 30;
    pub const DEMOLISH: u8 = 31;
    pub const SIEGE_WIN_GIVE_ITEM: u8 = 32;
    pub const SIEGE_LOSE_GIVE_ITEM: u8 = 33;
    pub const TAX_IN_KIND_RECEIPT: u8 = 34;
    pub const NATION_RELATION_FRIEND: u8 = 35;
    pub const NATION_RELATION_FRIEND_SUCCESS: u8 = 36;
    pub const NATION_RELATION_FRIEND_REJECT: u8 = 37;
    pub const NATION_RELATION_FRIEND_REJECT_ITEM: u8 = 38;
    pub const NATION_RELATION_HOSTILE: u8 = 39;
    pub const NATION_RELATION_HOSTILE_SUCCESS: u8 = 40;
    pub const NATION_RELATION_HOSTILE_REJECT: u8 = 41;
    pub const SIEGE_AUCTION_START: u8 = 42;
    pub const EXPEDITION_IMMIGRATION_REJECT: u8 = 43;
    pub const RESIDENT_BALANCE: u8 = 44;
}

pub mod result {
    pub const SUCCESS: u8 = 0;
    pub const INSUFFICIENT_COINS: u8 = 1;
    pub const CAN_NOT_BE_MAILED: u8 = 2;
    pub const INVALID_SLOT: u8 = 3;
    pub const CAN_NOT_FIND_MAIL: u8 = 4;
    pub const NO_ATTACHED_MONEY: u8 = 5;
    pub const NO_ATTACHED_ITEMS: u8 = 6;
    pub const INSUFFICIENT_COINS_2: u8 = 7;
    pub const RETURNS_NOT_ALLOWED: u8 = 8;
    pub const SUBJECT_LENGTH_LIMITED: u8 = 9;
    pub const TEXT_LENGTH_LIMITED: u8 = 10;
    pub const MAIL_NOT_INITIALIZED: u8 = 11;
    pub const INVALID_LETTER_FORMAT: u8 = 12;
    pub const INCORRECT_ITEM_INFORMATION: u8 = 13;
    pub const UNABLE_TO_ACCESS_SELECTED_MAIL: u8 = 14;
    pub const UNABLE_TO_FIND_RECIPIENT: u8 = 15;
    pub const NO_MAILBOX_NEARBY: u8 = 16;
    pub const INSUFFICIENT_COINS_3: u8 = 17;
    pub const FEATURE_UNDER_MAINTENANCE: u8 = 18;
    pub const BOUND_ITEM: u8 = 19;
    pub const NOT_AVAILABLE_IN_FREE_TRIAL: u8 = 20;
    pub const NOT_AVAILABLE_FOR_FREE_ACCOUNTS: u8 = 21;
    pub const LEVEL_TOO_LOW: u8 = 22;
    pub const YOU_MUST_PAY_FIRST: u8 = 23;
    pub const CHARACTERS_TRANSFERRING_SERVERS: u8 = 24;
    pub const SECONDARY_PASSWORD_REQUIRED: u8 = 25;
    pub const ITEM_LOCKED: u8 = 26;
    pub const CANNOT_PERFORM_SENSITIVE_ACTIONS: u8 = 27;
    pub const SENDING_MAILS_TOO_FREQUENT: u8 = 28;
    pub const SENT_SAME_MAIL_TOO_MANY_TIMES: u8 = 29;
    pub const MAIL_ERROR_OCCURRED: u8 = 30;
}

pub mod status {
    pub const UNREAD: u8 = 0;
    pub const READ: u8 = 1;
    pub const UNPAID: u8 = 2;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailSlot {
    pub container: u8,
    /// Zero-based physical slot, not the sorted UI index.
    pub slot: u8,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendMail {
    /// Only normal1, express2, and registered3 are accepted by the lane.
    pub mail_type: u8,
    pub receiver: String,
    pub title: String,
    pub text: String,
    pub money: i32,
    /// Whole stacks from bag(container2); split before attaching a partial stack.
    pub items: Vec<MailSlot>,
    pub doodad_obj_id: u32,
}

impl SendMail {
    /// Postage only, excluding the attached copper. The first item or money attachment is free.
    pub fn postage(&self) -> Option<i32> {
        let (base, extra) = match self.mail_type {
            mail_type::NORMAL | mail_type::REGISTERED => (50, 30),
            mail_type::EXPRESS => (100, 80),
            _ => return None,
        };
        let count = self.items.len().checked_add(usize::from(self.money > 0))?;
        i32::try_from(count.saturating_sub(1))
            .ok()?
            .checked_mul(extra)?
            .checked_add(base)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MailCommand {
    List,
    /// Present in 1.2, but the lane currently only logs it; List sends the entire snapshot.
    ListContinue,
    Read {
        is_sent: bool,
        mail_id: i64,
    },
    /// The lane silently refuses sent mail and received mail that still has attachments.
    Delete {
        is_sent: bool,
        mail_id: i64,
    },
    Send(SendMail),
    TakeMoney {
        mail_id: i64,
    },
    TakeItem {
        mail_id: i64,
        attachment: MailAttachment,
    },
    /// Claims money/items; keeps the mail. Native TakeAttachmentSequentially.
    TakeAll {
        mail_id: i64,
    },
    /// Claims money/items and deletes only if the lane completes every attachment.
    TakeAllAndDelete {
        mail_id: i64,
    },
    /// This opcode pays housing tax; it is not general cash-on-delivery support.
    PayHousingTax {
        mail_id: i64,
        auto_use_aa_point: bool,
    },
}

impl MailCommand {
    pub fn validate(&self) -> std::result::Result<(), &'static str> {
        match self {
            Self::Send(mail) => {
                if !matches!(mail.mail_type, 1..=3) {
                    return Err("invalid outgoing mail type");
                }
                if mail.receiver.is_empty() {
                    return Err("mail has no recipient");
                }
                if [&mail.receiver, &mail.title, &mail.text]
                    .iter()
                    .any(|s| s.len() > u16::MAX as usize)
                {
                    return Err("mail string exceeds the wire length");
                }
                if mail.receiver.len() + mail.title.len() + mail.text.len() > u16::MAX as usize - 61
                {
                    return Err("mail request exceeds the frame length");
                }
                if mail.doodad_obj_id == 0 || mail.doodad_obj_id > 0xff_ffff {
                    return Err("mail has no valid mailbox");
                }
                if mail.money < 0 {
                    return Err("negative mail money");
                }
                if mail.items.len() > MAX_ATTACHMENTS {
                    return Err("mail exceeds ten item attachments");
                }
                if mail.items.iter().any(|s| s.container != 2) {
                    return Err("mail attachments must be bag items");
                }
                if mail
                    .items
                    .iter()
                    .enumerate()
                    .any(|(i, s)| mail.items[..i].contains(s))
                {
                    return Err("duplicate mail attachment slot");
                }
            }
            Self::TakeItem { attachment, .. } if !attachment.valid() => {
                return Err("invalid mail attachment description");
            }
            command if command.mail_id().is_some_and(|id| id <= 0) => {
                return Err("invalid mail id");
            }
            _ => {}
        }
        Ok(())
    }

    pub fn mail_id(&self) -> Option<i64> {
        match self {
            Self::Read { mail_id, .. }
            | Self::Delete { mail_id, .. }
            | Self::TakeMoney { mail_id }
            | Self::TakeItem { mail_id, .. }
            | Self::TakeAll { mail_id }
            | Self::TakeAllAndDelete { mail_id }
            | Self::PayHousingTax { mail_id, .. } => Some(*mail_id),
            _ => None,
        }
    }

    pub fn opcode(&self) -> u16 {
        match self {
            Self::List => cs::CSListMailPacket,
            Self::ListContinue => cs::CSListMailContinuePacket,
            Self::Read { .. } => cs::CSReadMailPacket,
            Self::Delete { .. } => cs::CSDeleteMailPacket,
            Self::Send(_) => cs::CSSendMailPacket,
            Self::TakeMoney { .. } => cs::CSTakeAttachmentMoneyPacket,
            Self::TakeItem { .. } => cs::CSTakeAttachmentItemPacket,
            Self::TakeAll { .. } => cs::CSTakeAttachmentSequentially,
            Self::TakeAllAndDelete { .. } => cs::CSTakeAllAttachmentItemPacket,
            Self::PayHousingTax { .. } => cs::CSPayChargeMoneyPacket,
        }
    }

    pub fn write(&self, w: &mut Writer) {
        match self {
            Self::List | Self::ListContinue => {}
            Self::Read { is_sent, mail_id } => {
                w.bool(*is_sent).i64(*mail_id);
            }
            // Delete has the reverse field order from Read.
            Self::Delete { is_sent, mail_id } => {
                w.i64(*mail_id).bool(*is_sent);
            }
            Self::Send(mail) => {
                assert!(
                    mail.items.len() <= MAX_ATTACHMENTS,
                    "mail exceeds ten attachments"
                );
                let count = mail.items.len() + usize::from(mail.money > 0);
                w.u8(mail.mail_type)
                    .string(&mail.receiver)
                    .u32(0)
                    .string(&mail.title)
                    .string(&mail.text)
                    .u8(count as u8)
                    .i32(mail.money)
                    .i32(0)
                    .i32(0)
                    .i64(0);
                for i in 0..MAX_ATTACHMENTS {
                    let slot = mail.items.get(i).copied().unwrap_or_default();
                    w.u8(slot.container).u8(slot.slot);
                }
                w.bc(mail.doodad_obj_id);
            }
            Self::TakeItem {
                mail_id,
                attachment,
            } => {
                // Native397c5390 sends the complete Item.Write, then a zero target address.
                // The lane's shortened reader ignores the description after the item ID.
                w.i64(*mail_id).raw(&attachment.wire).u8(0).u8(0);
            }
            Self::TakeMoney { mail_id }
            | Self::TakeAll { mail_id }
            | Self::TakeAllAndDelete { mail_id } => {
                w.i64(*mail_id);
            }
            Self::PayHousingTax {
                mail_id,
                auto_use_aa_point,
            } => {
                w.i64(*mail_id).bool(*auto_use_aa_point);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnreadMail {
    pub sent: i32,
    pub received: i32,
    pub mia_received: i32,
    pub commercial_received: i32,
}

impl UnreadMail {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            sent: r.i32()?,
            received: r.i32()?,
            mia_received: r.i32()?,
            commercial_received: r.i32()?,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MailHeader {
    pub mail_id: i64,
    pub mail_type: u8,
    pub status: u8,
    pub title: String,
    pub sender_name: String,
    /// Includes money, not only item slots.
    pub attachments: u8,
    pub receiver_name: String,
    pub open_date: i64,
    pub returned: bool,
    pub extra: i64,
}

impl MailHeader {
    fn read(r: &mut Reader) -> Result<Self> {
        Ok(Self {
            mail_id: r.i64()?,
            mail_type: r.u8()?,
            status: r.u8()?,
            title: r.string()?,
            sender_name: r.string()?,
            attachments: r.u8()?,
            receiver_name: r.string()?,
            open_date: r.i64()?,
            returned: r.bool()?,
            extra: r.i64()?,
        })
    }
}

/// A complete original Item.Write. Retains skipped detail bytes for a native claim request.
#[derive(Clone, PartialEq)]
pub struct MailAttachment {
    pub item: Item,
    wire: Vec<u8>,
}

impl std::fmt::Debug for MailAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailAttachment")
            .field("item", &self.item)
            .finish_non_exhaustive()
    }
}

impl MailAttachment {
    fn read(r: &mut Reader) -> Result<Option<Self>> {
        let before = r.rest();
        let item = Item::read(r)?;
        let wire = before[..before.len() - r.remaining()].to_vec();
        Ok(item.map(|item| Self { item, wire }))
    }

    /// Accept a complete server Item.Write, including the detail block and timestamps.
    pub fn from_wire(wire: &[u8]) -> Result<Self> {
        let mut r = Reader::new(wire);
        let attachment = Self::read(&mut r)?
            .ok_or_else(|| WireError::Invalid("empty mail attachment".into()))?;
        if !r.is_empty() || attachment.item.id == 0 || attachment.item.count <= 0 {
            return Err(WireError::Invalid(
                "invalid mail attachment description".into(),
            ));
        }
        Ok(attachment)
    }

    fn valid(&self) -> bool {
        Self::from_wire(&self.wire).is_ok_and(|parsed| parsed.item == self.item)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MailBody {
    pub mail_id: i64,
    pub mail_type: u8,
    pub receiver_name: String,
    pub title: String,
    pub text: String,
    pub money: i32,
    pub billing_amount: i32,
    pub money_amount2: i32,
    pub send_date: i64,
    pub recv_date: i64,
    pub open_date: i64,
    /// Ten physical attachment slots, including empty slots. Lua indices are one-based.
    pub items: [Option<MailAttachment>; MAX_ATTACHMENTS],
}

impl MailBody {
    fn read(r: &mut Reader) -> Result<Self> {
        let mut body = Self {
            mail_id: r.i64()?,
            mail_type: r.u8()?,
            receiver_name: r.string()?,
            title: r.string()?,
            text: r.string()?,
            money: r.i32()?,
            billing_amount: r.i32()?,
            money_amount2: r.i32()?,
            send_date: r.i64()?,
            recv_date: r.i64()?,
            open_date: r.i64()?,
            ..Self::default()
        };
        for slot in &mut body.items {
            *slot = MailAttachment::read(r)?;
        }
        Ok(body)
    }

    fn attachment_count(&self) -> u8 {
        self.items.iter().filter(|i| i.is_some()).count() as u8
            + u8::from(self.money != 0)
            + u8::from(self.billing_amount != 0)
            + u8::from(self.money_amount2 != 0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MailItemLocation {
    pub item_id: u64,
    pub container: u8,
    pub slot: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MailEvent {
    Failed {
        code: u8,
        slots: [MailSlot; MAX_ATTACHMENTS],
        money: bool,
    },
    Unread(UnreadMail),
    Sent {
        header: MailHeader,
        slots: [MailSlot; MAX_ATTACHMENTS],
    },
    Got {
        header: MailHeader,
        unread: UnreadMail,
        body: Option<MailBody>,
        is_cancel: bool,
    },
    List {
        is_sent: bool,
        headers: Vec<MailHeader>,
    },
    ListEnd {
        total_headers: i32,
        total_bodies: i32,
    },
    Body {
        is_prepare: bool,
        is_sent: bool,
        body: MailBody,
        open_date_modified: bool,
        unread: UnreadMail,
    },
    ReceiverOpened {
        mail_id: i64,
        open_date: i64,
    },
    AttachmentTaken {
        mail_id: i64,
        money: bool,
        aa_point: bool,
        sequential: bool,
        items: Vec<MailItemLocation>,
    },
    ChargePaid {
        mail_id: i64,
    },
    Deleted {
        is_sent: bool,
        mail_id: i64,
        unread_modified: bool,
        unread: UnreadMail,
    },
    Returned {
        mail_id: i64,
        header: MailHeader,
    },
    StatusUpdated {
        is_sent: bool,
        mail_id: i64,
        status: u8,
    },
    Removed {
        is_sent: bool,
        mail_id: i64,
    },
    /// Local acknowledgement of sending a request, never of server success.
    Requested(MailCommand),
    RequestRejected {
        command: MailCommand,
        message: String,
    },
}

impl MailEvent {
    pub fn handles(opcode: u16) -> bool {
        matches!(opcode, 0x115..=0x11f | 0x121..=0x123)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Failed { .. } => "SCMailFailed",
            Self::Unread(_) => "SCCountUnreadMail",
            Self::Sent { .. } => "SCMailSent",
            Self::Got { .. } => "SCGotMail",
            Self::List { .. } => "SCMailList",
            Self::ListEnd { .. } => "SCMailListEnd",
            Self::Body { .. } => "SCMailBody",
            Self::ReceiverOpened { .. } => "SCMailReceiverOpened",
            Self::AttachmentTaken { .. } => "SCAttachmentTaken",
            Self::ChargePaid { .. } => "SCChargeMoneyPaid",
            Self::Deleted { .. } => "SCMailDeleted",
            Self::Returned { .. } => "SCMailReturned",
            Self::StatusUpdated { .. } => "SCMailStatusUpdated",
            Self::Removed { .. } => "SCMailRemoved",
            Self::Requested(_) => "MailRequested",
            Self::RequestRejected { .. } => "MailRequestRejected",
        }
    }

    pub fn read(opcode: u16, r: &mut Reader) -> Result<Self> {
        let read_slots = |r: &mut Reader| -> Result<[MailSlot; MAX_ATTACHMENTS]> {
            let mut slots = [MailSlot::default(); MAX_ATTACHMENTS];
            for slot in &mut slots {
                *slot = MailSlot {
                    container: r.u8()?,
                    slot: r.u8()?,
                };
            }
            Ok(slots)
        };
        Ok(match opcode {
            sc::SCMailFailedPacket => Self::Failed {
                code: r.u8()?,
                slots: read_slots(r)?,
                money: r.bool()?,
            },
            sc::SCCountUnreadMailPacket => Self::Unread(UnreadMail::read(r)?),
            sc::SCMailSentPacket => Self::Sent {
                header: MailHeader::read(r)?,
                slots: read_slots(r)?,
            },
            sc::SCGotMailPacket => {
                let header = MailHeader::read(r)?;
                let unread = UnreadMail::read(r)?;
                let body = if r.bool()? {
                    Some(MailBody::read(r)?)
                } else {
                    None
                };
                if body.as_ref().is_some_and(|b| b.mail_id != header.mail_id) {
                    return Err(WireError::Invalid(
                        "got-mail header/body id mismatch".into(),
                    ));
                }
                Self::Got {
                    header,
                    unread,
                    body,
                    is_cancel: r.bool()?,
                }
            }
            sc::SCMailListPacket => {
                let is_sent = r.bool()?;
                let count = r.i32()?;
                if count < 0 || count as usize > r.remaining() / 34 {
                    return Err(WireError::Invalid(format!(
                        "invalid mail header count {count}"
                    )));
                }
                let mut headers = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    headers.push(MailHeader::read(r)?);
                }
                Self::List { is_sent, headers }
            }
            sc::SCMailListEndPacket => {
                let (total_headers, total_bodies) = (r.i32()?, r.i32()?);
                if total_headers < 0 || total_bodies < 0 {
                    return Err(WireError::Invalid("negative mail list totals".into()));
                }
                Self::ListEnd {
                    total_headers,
                    total_bodies,
                }
            }
            sc::SCMailBodyPacket => Self::Body {
                is_prepare: r.bool()?,
                is_sent: r.bool()?,
                body: MailBody::read(r)?,
                open_date_modified: r.bool()?,
                unread: UnreadMail::read(r)?,
            },
            sc::SCMailReceiverOpenedPacket => Self::ReceiverOpened {
                mail_id: r.i64()?,
                open_date: r.i64()?,
            },
            sc::SCAttachmentTakenPacket => {
                let (mail_id, money, aa_point, sequential, count) =
                    (r.i64()?, r.bool()?, r.bool()?, r.bool()?, r.u8()?);
                if count as usize > MAX_ATTACHMENTS {
                    return Err(WireError::Invalid("mail claim count exceeds ten".into()));
                }
                let mut items = Vec::with_capacity(count as usize);
                for i in 0..MAX_ATTACHMENTS {
                    let item = MailItemLocation {
                        item_id: r.u64()?,
                        container: r.u8()?,
                        slot: r.u8()?,
                    };
                    if i < count as usize {
                        if item.item_id == 0
                            || items
                                .iter()
                                .any(|old: &MailItemLocation| old.item_id == item.item_id)
                        {
                            return Err(WireError::Invalid(
                                "empty or duplicate mail claim item".into(),
                            ));
                        }
                        items.push(item);
                    } else if item != MailItemLocation::default() {
                        return Err(WireError::Invalid("nonempty mail claim padding".into()));
                    }
                }
                Self::AttachmentTaken {
                    mail_id,
                    money,
                    aa_point,
                    sequential,
                    items,
                }
            }
            sc::SCChargeMoneyPaidPacket => Self::ChargePaid { mail_id: r.i64()? },
            sc::SCMailDeletedPacket => Self::Deleted {
                is_sent: r.bool()?,
                mail_id: r.i64()?,
                unread_modified: r.bool()?,
                unread: UnreadMail::read(r)?,
            },
            sc::SCMailReturnedPacket => Self::Returned {
                mail_id: r.i64()?,
                header: MailHeader::read(r)?,
            },
            sc::SCMailStatusUpdatedPacket => Self::StatusUpdated {
                is_sent: r.bool()?,
                mail_id: r.i64()?,
                status: r.u8()?,
            },
            sc::SCMailRemovedPacket => Self::Removed {
                is_sent: r.bool()?,
                mail_id: r.i64()?,
            },
            _ => {
                return Err(WireError::Invalid(format!(
                    "unknown mail opcode {opcode:03x}"
                )));
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CachedMailBody {
    pub is_sent: bool,
    pub body: MailBody,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MailFailure {
    /// Failed packets carry no request ID; only Send is correlated by this lane.
    pub command: Option<MailCommand>,
    pub code: Option<u8>,
    pub slots: Vec<MailSlot>,
    pub money: bool,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(feature = "bevy", derive(bevy::ecs::resource::Resource))]
pub struct Mailbox {
    pub received: Vec<MailHeader>,
    pub sent: Vec<MailHeader>,
    pub bodies: Vec<CachedMailBody>,
    pub unread: UnreadMail,
    pub unread_loaded: bool,
    pub loaded: bool,
    pub listing: bool,
    /// An attachment ACK arrived before the body was cached. List/read refresh the counts.
    pub dirty: bool,
    pub total_headers: i32,
    pub total_bodies: i32,
    pub pending: Vec<MailCommand>,
    pub last_error: Option<MailFailure>,
}

impl Mailbox {
    pub fn headers(&self, is_sent: bool) -> &[MailHeader] {
        if is_sent { &self.sent } else { &self.received }
    }
    fn headers_mut(&mut self, is_sent: bool) -> &mut Vec<MailHeader> {
        if is_sent {
            &mut self.sent
        } else {
            &mut self.received
        }
    }
    pub fn header(&self, is_sent: bool, id: i64) -> Option<&MailHeader> {
        self.headers(is_sent).iter().find(|h| h.mail_id == id)
    }
    pub fn body(&self, is_sent: bool, id: i64) -> Option<&MailBody> {
        self.bodies
            .iter()
            .find(|b| b.is_sent == is_sent && b.body.mail_id == id)
            .map(|b| &b.body)
    }
    fn upsert_header(&mut self, is_sent: bool, header: &MailHeader) {
        let headers = self.headers_mut(is_sent);
        if let Some(old) = headers.iter_mut().find(|h| h.mail_id == header.mail_id) {
            *old = header.clone();
        } else {
            headers.push(header.clone());
        }
    }
    fn upsert_body(&mut self, is_sent: bool, body: &MailBody) {
        if let Some(old) = self
            .bodies
            .iter_mut()
            .find(|b| b.is_sent == is_sent && b.body.mail_id == body.mail_id)
        {
            old.body = body.clone();
        } else {
            self.bodies.push(CachedMailBody {
                is_sent,
                body: body.clone(),
            });
        }
    }
    fn remove(&mut self, is_sent: bool, id: i64) {
        self.headers_mut(is_sent).retain(|h| h.mail_id != id);
        self.bodies
            .retain(|b| b.is_sent != is_sent || b.body.mail_id != id);
        self.pending.retain(|p| p.mail_id() != Some(id));
    }
    fn set_unread(&mut self, unread: UnreadMail) {
        self.unread = unread;
        self.unread_loaded = true;
    }

    pub fn apply(&mut self, event: &Event) -> bool {
        let Event::Mail(event) = event else {
            if matches!(
                event,
                Event::CharacterSelected(_) | Event::LeftWorld { .. } | Event::Disconnected { .. }
            ) {
                let changed = *self != Self::default();
                *self = Self::default();
                return changed;
            }
            return false;
        };
        match event {
            MailEvent::Requested(command) => {
                if matches!(command, MailCommand::List) {
                    self.received.clear();
                    self.sent.clear();
                    self.bodies.clear();
                    self.loaded = false;
                    self.listing = true;
                    self.dirty = false;
                    self.total_headers = 0;
                    self.total_bodies = 0;
                }
                if !self.pending.contains(command) {
                    self.pending.push(command.clone());
                }
                self.last_error = None;
            }
            MailEvent::RequestRejected { command, message } => {
                self.last_error = Some(MailFailure {
                    command: Some(command.clone()),
                    message: message.clone(),
                    ..MailFailure::default()
                });
            }
            MailEvent::Failed { code, slots, money } => {
                let command = self
                    .pending
                    .iter()
                    .position(|p| matches!(p, MailCommand::Send(_)))
                    .map(|i| self.pending.remove(i));
                self.last_error = Some(MailFailure {
                    command,
                    code: Some(*code),
                    slots: slots.to_vec(),
                    money: *money,
                    message: format!("mail request failed ({code})"),
                });
            }
            MailEvent::Unread(unread) => self.set_unread(*unread),
            MailEvent::List { is_sent, headers } => {
                for h in headers {
                    self.upsert_header(*is_sent, h);
                }
            }
            MailEvent::ListEnd {
                total_headers,
                total_bodies,
            } => {
                self.total_headers = *total_headers;
                self.total_bodies = *total_bodies;
                self.loaded = true;
                self.listing = false;
                self.pending
                    .retain(|p| !matches!(p, MailCommand::List | MailCommand::ListContinue));
            }
            MailEvent::Sent { header, .. } => {
                self.upsert_header(true, header);
                if let Some(i) = self
                    .pending
                    .iter()
                    .position(|p| matches!(p, MailCommand::Send(_)))
                {
                    self.pending.remove(i);
                }
                self.last_error = None;
            }
            MailEvent::Got {
                header,
                unread,
                body,
                ..
            } => {
                self.upsert_header(false, header);
                self.set_unread(*unread);
                if let Some(body) = body {
                    self.upsert_body(false, body);
                }
            }
            MailEvent::Body {
                is_sent,
                body,
                open_date_modified,
                unread,
                ..
            } => {
                self.upsert_body(*is_sent, body);
                self.set_unread(*unread);
                if let Some(header) = self
                    .headers_mut(*is_sent)
                    .iter_mut()
                    .find(|h| h.mail_id == body.mail_id)
                {
                    header.attachments = body.attachment_count();
                    if *open_date_modified {
                        header.open_date = body.open_date;
                    }
                }
                self.pending.retain(|p| !matches!(p, MailCommand::Read { is_sent: sent, mail_id } if sent == is_sent && *mail_id == body.mail_id));
                if !*is_sent && body.attachment_count() == 0 {
                    self.pending.retain(|p| !matches!(p, MailCommand::TakeAll { mail_id } if *mail_id == body.mail_id));
                }
            }
            MailEvent::ReceiverOpened { mail_id, open_date } => {
                if let Some(header) = self.sent.iter_mut().find(|h| h.mail_id == *mail_id) {
                    header.open_date = *open_date;
                }
                if let Some(body) = self
                    .bodies
                    .iter_mut()
                    .find(|b| b.is_sent && b.body.mail_id == *mail_id)
                {
                    body.body.open_date = *open_date;
                }
            }
            MailEvent::AttachmentTaken {
                mail_id,
                money,
                aa_point,
                items,
                ..
            } => {
                if let Some(cache) = self
                    .bodies
                    .iter_mut()
                    .find(|b| !b.is_sent && b.body.mail_id == *mail_id)
                {
                    if *money {
                        cache.body.money = 0;
                    }
                    if *aa_point {
                        cache.body.money_amount2 = 0;
                    }
                    for slot in &mut cache.body.items {
                        if slot
                            .as_ref()
                            .is_some_and(|it| items.iter().any(|taken| taken.item_id == it.item.id))
                        {
                            *slot = None;
                        }
                    }
                    let count = cache.body.attachment_count();
                    if let Some(header) = self.received.iter_mut().find(|h| h.mail_id == *mail_id) {
                        header.attachments = count;
                    }
                } else {
                    self.dirty = true;
                }
                let remaining = self.body(false, *mail_id).map(MailBody::attachment_count);
                self.pending.retain(|p| match p {
                    MailCommand::TakeMoney { mail_id: id } => id != mail_id || !*money,
                    MailCommand::TakeItem {
                        mail_id: id,
                        attachment,
                    } => id != mail_id || !items.iter().any(|i| i.item_id == attachment.item.id),
                    MailCommand::TakeAll { mail_id: id } => id != mail_id || remaining != Some(0),
                    _ => true,
                });
            }
            MailEvent::ChargePaid { mail_id } => {
                self.pending.retain(|p| !matches!(p, MailCommand::PayHousingTax { mail_id: id, .. } if id == mail_id));
                self.dirty = true;
            }
            MailEvent::Deleted {
                is_sent,
                mail_id,
                unread,
                ..
            } => {
                self.remove(*is_sent, *mail_id);
                self.set_unread(*unread);
            }
            MailEvent::Removed { is_sent, mail_id } => self.remove(*is_sent, *mail_id),
            MailEvent::Returned { mail_id, header } => {
                self.remove(false, *mail_id);
                self.upsert_header(true, header);
            }
            MailEvent::StatusUpdated {
                is_sent,
                mail_id,
                status,
            } => {
                if let Some(header) = self
                    .headers_mut(*is_sent)
                    .iter_mut()
                    .find(|h| h.mail_id == *mail_id)
                {
                    header.status = *status;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{ClientMsg, ServerMsg};

    fn header(w: &mut Writer, id: i64, attachments: u8) {
        w.i64(id)
            .u8(3)
            .u8(0)
            .string("Title")
            .string("Sender")
            .u8(attachments)
            .string("Receiver")
            .i64(50)
            .bool(false)
            .i64(77);
    }
    fn unread(w: &mut Writer) {
        w.i32(1).i32(2).i32(3).i32(4);
    }
    fn item(w: &mut Writer, id: u64, detail: u8) {
        w.u32(100).u64(id).u8(2).u8(0).i32(3).u8(detail);
        match detail {
            0 => {}
            1 => {
                w.u32(200).u8(80).u16(0).u32(201).i64(202).u32(203);
                for gem in 204..211 {
                    w.u32(gem);
                }
                w.u16(211).u16(212);
            }
            4 => {
                w.raw(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
            }
            _ => panic!(),
        }
        w.i64(11).i32(12).u32(13).u8(1).i64(14).i64(15);
    }
    fn body(w: &mut Writer, id: i64, attachments: bool) {
        w.i64(id)
            .u8(3)
            .string("Receiver")
            .string("Title")
            .string("Text")
            .i32(5)
            .i32(0)
            .i32(0)
            .i64(10)
            .i64(20)
            .i64(30);
        for i in 0..10 {
            if attachments && (i == 0 || i == 9) {
                item(w, 1000 + i, if i == 0 { 1 } else { 4 });
            } else {
                w.u32(0);
            }
        }
    }
    fn bytes(write: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::new();
        write(&mut w);
        w.finish()
    }
    fn decode(op: u16, bytes: &[u8]) -> MailEvent {
        let mut r = Reader::new(bytes);
        let event = MailEvent::read(op, &mut r).unwrap();
        assert!(r.is_empty(), "trailing bytes in {op:03x}");
        event
    }
    fn batch(packets: &[(u16, Vec<u8>)]) -> Result<Vec<ServerMsg>> {
        let mut w = Writer::new();
        for (opcode, body) in packets {
            w.u16(0).u16(*opcode).raw(body);
        }
        w.u16(0).u16(sc::SCOnOffSnowPacket).bool(true);
        let raw = w.finish();
        let len = raw.len() as u16;
        let mut w = Writer::new();
        w.raw(&[0xdd, 4])
            .u16(packets.len() as u16 + 1)
            .u8(1)
            .u16(len)
            .u16(!len)
            .raw(&raw);
        match ServerMsg::decode(&w.finish())? {
            ServerMsg::Batch(batch) => Ok(batch),
            _ => panic!(),
        }
    }

    #[test]
    fn every_mail_packet_preserves_dd04_boundary_including_ten_fixed_slots() {
        let packets = vec![
            (
                0x115,
                bytes(|w| {
                    w.u8(15).raw(&[0; 20]).bool(false);
                }),
            ),
            (0x116, bytes(unread)),
            (
                0x117,
                bytes(|w| {
                    header(w, 1, 2);
                    w.raw(&[0; 20]);
                }),
            ),
            (
                0x118,
                bytes(|w| {
                    header(w, 2, 2);
                    unread(w);
                    w.bool(true);
                    body(w, 2, true);
                    w.bool(false);
                }),
            ),
            (
                0x118,
                bytes(|w| {
                    header(w, 3, 0);
                    unread(w);
                    w.bool(false).bool(true);
                }),
            ),
            (
                0x119,
                bytes(|w| {
                    w.bool(false).i32(2);
                    header(w, 4, 0);
                    header(w, 5, 0);
                }),
            ),
            (
                0x11a,
                bytes(|w| {
                    w.i32(2).i32(0);
                }),
            ),
            (
                0x11b,
                bytes(|w| {
                    w.bool(false).bool(false);
                    body(w, 6, true);
                    w.bool(true);
                    unread(w);
                }),
            ),
            (
                0x11c,
                bytes(|w| {
                    w.i64(7).i64(90);
                }),
            ),
            (
                0x11d,
                bytes(|w| {
                    w.i64(6)
                        .bool(true)
                        .bool(false)
                        .bool(true)
                        .u8(1)
                        .u64(1000)
                        .u8(4)
                        .u8(0)
                        .raw(&[0; 90]);
                }),
            ),
            (
                0x11e,
                bytes(|w| {
                    w.i64(8);
                }),
            ),
            (
                0x11f,
                bytes(|w| {
                    w.bool(false).i64(9).bool(false);
                    unread(w);
                }),
            ),
            (
                0x121,
                bytes(|w| {
                    w.i64(10);
                    header(w, 11, 0);
                }),
            ),
            (
                0x122,
                bytes(|w| {
                    w.bool(true).i64(12).u8(1);
                }),
            ),
            (
                0x123,
                bytes(|w| {
                    w.bool(true).i64(13);
                }),
            ),
        ];
        let result = batch(&packets).unwrap();
        assert_eq!(result.len(), packets.len() + 1);
        assert_eq!(result.last(), Some(&ServerMsg::Snow(true)));
        for (i, (opcode, body)) in packets.iter().enumerate() {
            assert_eq!(result[i], ServerMsg::Mail(decode(*opcode, body)));
        }
        let ServerMsg::Mail(MailEvent::Body { body, .. }) = &result[7] else {
            panic!()
        };
        assert!(matches!(
            body.items[0].as_ref().unwrap().item.detail,
            crate::model::ItemDetail::Equip(_)
        ));
        assert!(matches!(
            body.items[9].as_ref().unwrap().item.detail,
            crate::model::ItemDetail::Other(4)
        ));
    }

    #[test]
    fn malformed_mail_counts_and_truncation_are_errors() {
        for count in [-1, i32::MAX, 1] {
            assert!(
                MailEvent::read(
                    0x119,
                    &mut Reader::new(&bytes(|w| {
                        w.bool(false).i32(count);
                    }))
                )
                .is_err()
            );
        }
        for count in [-1, i32::MAX] {
            let raw = bytes(|w| {
                w.i32(count).i32(0);
            });
            if count < 0 {
                assert!(MailEvent::read(0x11a, &mut Reader::new(&raw)).is_err());
            }
        }
        let too_many = bytes(|w| {
            w.i64(1).raw(&[0; 3]).u8(11).raw(&[0; 100]);
        });
        assert!(MailEvent::read(0x11d, &mut Reader::new(&too_many)).is_err());
        let padding = bytes(|w| {
            w.i64(1).raw(&[0; 3]).u8(0).u64(2).u8(0).u8(0).raw(&[0; 90]);
        });
        assert!(MailEvent::read(0x11d, &mut Reader::new(&padding)).is_err());
        let duplicate = bytes(|w| {
            w.i64(1).raw(&[0; 3]).u8(2);
            for _ in 0..2 {
                w.u64(2).u8(4).u8(0);
            }
            w.raw(&[0; 80]);
        });
        assert!(MailEvent::read(0x11d, &mut Reader::new(&duplicate)).is_err());
        let raw = bytes(|w| {
            w.bool(false).bool(false);
            body(w, 1, true);
            w.bool(false);
            unread(w);
        });
        for n in 0..raw.len() {
            assert!(
                MailEvent::read(0x11b, &mut Reader::new(&raw[..n])).is_err(),
                "truncation {n}"
            );
        }
    }

    #[test]
    fn commands_match_native_orders_and_retained_complete_item_details() {
        let read = ClientMsg::Mail(MailCommand::Read {
            is_sent: true,
            mail_id: 0x0102,
        });
        assert_eq!(
            (read.opcode(), read.encode_body()),
            (0x09c, vec![1, 2, 1, 0, 0, 0, 0, 0, 0])
        );
        let delete = ClientMsg::Mail(MailCommand::Delete {
            is_sent: true,
            mail_id: 0x0102,
        });
        assert_eq!(
            (delete.opcode(), delete.encode_body()),
            (0x0a1, vec![2, 1, 0, 0, 0, 0, 0, 0, 1])
        );
        for detail in [0, 1, 4] {
            let raw = bytes(|w| item(w, 888, detail));
            let attachment = MailAttachment::from_wire(&raw).unwrap();
            let command = MailCommand::TakeItem {
                mail_id: 42,
                attachment,
            };
            assert!(command.validate().is_ok());
            let packet = ClientMsg::Mail(command);
            let mut expected = 42i64.to_le_bytes().to_vec();
            expected.extend(raw);
            expected.extend([0, 0]);
            assert_eq!((packet.opcode(), packet.encode_body()), (0x09d, expected));
        }
        let commands = [
            MailCommand::List,
            MailCommand::ListContinue,
            MailCommand::TakeMoney { mail_id: 1 },
            MailCommand::TakeAll { mail_id: 1 },
            MailCommand::TakeAllAndDelete { mail_id: 1 },
            MailCommand::PayHousingTax {
                mail_id: 1,
                auto_use_aa_point: true,
            },
        ];
        for (command, (opcode, len)) in commands.into_iter().zip([
            (0x9a, 0),
            (0x9b, 0),
            (0x9e, 8),
            (0x9f, 8),
            (0x139, 8),
            (0xa0, 9),
        ]) {
            let packet = ClientMsg::Mail(command);
            assert_eq!((packet.opcode(), packet.encode_body().len()), (opcode, len));
        }
    }

    #[test]
    fn send_has_ten_fixed_bag_slots_postage_and_validation() {
        let mut mail = SendMail {
            mail_type: 3,
            receiver: "Receiver".into(),
            title: "Title".into(),
            text: "Text".into(),
            money: 5,
            items: vec![MailSlot {
                container: 2,
                slot: 7,
            }],
            doodad_obj_id: 0x010203,
        };
        assert_eq!(mail.postage(), Some(80));
        let command = MailCommand::Send(mail.clone());
        assert!(command.validate().is_ok());
        let packet = ClientMsg::Mail(command);
        assert_eq!(packet.opcode(), 0x98);
        let raw = packet.encode_body();
        let mut r = Reader::new(&raw);
        assert_eq!(
            (r.u8().unwrap(), r.string().unwrap(), r.u32().unwrap()),
            (3, "Receiver".into(), 0)
        );
        assert_eq!(
            (r.string().unwrap(), r.string().unwrap(), r.u8().unwrap()),
            ("Title".into(), "Text".into(), 2)
        );
        assert_eq!(
            (
                r.i32().unwrap(),
                r.i32().unwrap(),
                r.i32().unwrap(),
                r.i64().unwrap()
            ),
            (5, 0, 0, 0)
        );
        assert_eq!((r.u8().unwrap(), r.u8().unwrap()), (2, 7));
        for _ in 1..10 {
            assert_eq!(r.u16().unwrap(), 0);
        }
        assert_eq!(r.bc().unwrap(), 0x010203);
        assert!(r.is_empty());
        mail.items.push(mail.items[0]);
        assert!(MailCommand::Send(mail.clone()).validate().is_err());
        mail.items = vec![MailSlot {
            container: 3,
            slot: 0,
        }];
        assert!(MailCommand::Send(mail.clone()).validate().is_err());
        mail.items.clear();
        mail.money = 0;
        assert_eq!(mail.postage(), Some(50));
        mail.mail_type = 2;
        assert_eq!(mail.postage(), Some(100));
        mail.items = vec![
            MailSlot {
                container: 2,
                slot: 0
            };
            11
        ];
        assert!(MailCommand::Send(mail.clone()).validate().is_err());
        assert!(MailCommand::TakeAll { mail_id: 0 }.validate().is_err());
        mail.items.clear();
        mail.text = "x".repeat(65_500);
        mail.title = "y".repeat(50);
        assert!(MailCommand::Send(mail).validate().is_err());
    }

    #[test]
    fn list_chunks_and_server_confirmed_claims_do_not_fabricate_state() {
        let mut mailbox = Mailbox::default();
        let list = |sent, id| {
            decode(
                0x119,
                &bytes(|w| {
                    w.bool(sent).i32(1);
                    header(w, id, 3);
                }),
            )
        };
        mailbox.apply(&Event::Mail(MailEvent::Requested(MailCommand::List)));
        for event in [list(false, 1), list(true, 2), list(false, 3)] {
            mailbox.apply(&Event::Mail(event));
        }
        assert_eq!((mailbox.received.len(), mailbox.sent.len()), (2, 1));
        assert!(!mailbox.loaded && mailbox.listing);
        mailbox.apply(&Event::Mail(MailEvent::ListEnd {
            total_headers: 3,
            total_bodies: 0,
        }));
        assert!(mailbox.loaded && !mailbox.listing && mailbox.pending.is_empty());
        mailbox.apply(&Event::Mail(decode(
            0x11b,
            &bytes(|w| {
                w.bool(false).bool(false);
                body(w, 1, true);
                w.bool(true);
                unread(w);
            }),
        )));
        let command = MailCommand::TakeAll { mail_id: 1 };
        mailbox.apply(&Event::Mail(MailEvent::Requested(command.clone())));
        assert_eq!(mailbox.header(false, 1).unwrap().attachments, 3);
        assert!(mailbox.body(false, 1).unwrap().items[0].is_some());
        let claim = |money, items| {
            Event::Mail(MailEvent::AttachmentTaken {
                mail_id: 1,
                money,
                aa_point: false,
                sequential: true,
                items,
            })
        };
        mailbox.apply(&claim(true, vec![]));
        assert_eq!(mailbox.header(false, 1).unwrap().attachments, 2);
        mailbox.apply(&claim(
            true,
            vec![MailItemLocation {
                item_id: 1000,
                container: 4,
                slot: 0,
            }],
        ));
        assert_eq!(mailbox.header(false, 1).unwrap().attachments, 1);
        assert!(mailbox.pending.contains(&command));
        mailbox.apply(&claim(
            true,
            vec![MailItemLocation {
                item_id: 1009,
                container: 4,
                slot: 9,
            }],
        ));
        assert_eq!(mailbox.header(false, 1).unwrap().attachments, 0);
        assert!(!mailbox.pending.contains(&command));
        assert!(mailbox.header(false, 1).is_some());
        mailbox.apply(&Event::Mail(MailEvent::Requested(MailCommand::Delete {
            is_sent: false,
            mail_id: 1,
        })));
        assert!(mailbox.header(false, 1).is_some());
        mailbox.apply(&Event::Mail(MailEvent::Deleted {
            is_sent: false,
            mail_id: 1,
            unread_modified: false,
            unread: UnreadMail::default(),
        }));
        assert!(mailbox.header(false, 1).is_none() && mailbox.body(false, 1).is_none());
        mailbox.apply(&Event::Mail(MailEvent::Requested(MailCommand::List)));
        assert!(mailbox.received.is_empty() && mailbox.sent.is_empty() && !mailbox.loaded);
        mailbox.apply(&Event::Mail(MailEvent::ListEnd {
            total_headers: 0,
            total_bodies: 0,
        }));
        assert!(mailbox.loaded && mailbox.received.is_empty());
        mailbox.apply(&Event::Disconnected {
            message: "test".into(),
        });
        assert_eq!(mailbox, Mailbox::default());
    }

    #[test]
    fn failures_keep_data_and_requests_without_ack_remain_pending() {
        let mut mailbox = Mailbox::default();
        let send = MailCommand::Send(SendMail {
            mail_type: 3,
            receiver: "Nobody".into(),
            doodad_obj_id: 42,
            ..SendMail::default()
        });
        mailbox.apply(&Event::Mail(MailEvent::Requested(send.clone())));
        mailbox.apply(&Event::Mail(MailEvent::Failed {
            code: 15,
            slots: [MailSlot::default(); 10],
            money: false,
        }));
        assert_eq!(mailbox.last_error.as_ref().unwrap().command, Some(send));
        assert!(mailbox.pending.is_empty());
        let take = MailCommand::TakeAll { mail_id: 1 };
        mailbox.apply(&Event::Mail(MailEvent::Requested(take.clone())));
        mailbox.apply(&Event::Mail(MailEvent::AttachmentTaken {
            mail_id: 1,
            money: true,
            aa_point: false,
            sequential: true,
            items: vec![],
        }));
        assert!(mailbox.dirty && mailbox.pending.contains(&take));
        let mut empty_body = MailBody {
            mail_id: 1,
            ..MailBody::default()
        };
        empty_body.money = 0;
        mailbox.apply(&Event::Mail(MailEvent::Body {
            is_prepare: false,
            is_sent: false,
            body: empty_body,
            open_date_modified: false,
            unread: UnreadMail::default(),
        }));
        assert!(!mailbox.pending.contains(&take));
        mailbox.apply(&Event::LeftWorld { target: 0 });
        assert_eq!(mailbox, Mailbox::default());
    }

    #[test]
    fn sent_mail_escrow_remove_uses_unique_id_without_touching_other_slots() {
        use crate::items::{ItemTask, ItemTasks, container};
        let mut inventory = crate::state::Inventory::default();
        inventory.bag = vec![None; 6];
        inventory.bag[4] = Some(Item {
            id: 77,
            template_id: 100,
            count: 1,
            ..Item::default()
        });
        inventory.bag[0] = Some(Item {
            id: 88,
            template_id: 101,
            count: 1,
            ..Item::default()
        });
        inventory.equipment.slots[19] = Item::template_only(200);
        let event = Event::ItemsChanged(ItemTasks {
            reason: 47,
            tasks: vec![ItemTask::Remove {
                container: container::MAIL,
                slot: 0,
                item_id: 77,
                template_id: 100,
            }],
            ..ItemTasks::default()
        });
        assert!(inventory.apply(&event));
        assert!(inventory.bag[4].is_none());
        assert_eq!(inventory.bag[0].as_ref().unwrap().id, 88);
        assert!(inventory.equipment.slots[19].is_some());
    }
}
