
use super::{ChangeFacts, FieldStamp, SharedView, TableView, ViewIdentity};
use replay_spool::{Placeholder, Reader, Spooled, Writer};
use std::{
    io,
    sync::{Arc, Weak},
};

impl Spooled for TableView {
    const WEAKLY: bool = true;

    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        match self {
            Self::Complete(rows) => {
                out.tag(0)?;
                rows.spool(out)
            }
            Self::Truncated { rows, missing } => {
                out.tag(1)?;
                rows.spool(out)?;
                missing.spool(out)
            }
            Self::Unavailable { reason } => {
                out.tag(2)?;
                reason.spool(out)
            }
            Self::NotRecorded { mode } => {
                out.tag(3)?;
                mode.spool(out)
            }
        }
    }

    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(match input.tag()? {
            0 => Self::Complete(Spooled::unspool(input)?),
            1 => Self::Truncated {
                rows: Spooled::unspool(input)?,
                missing: Spooled::unspool(input)?,
            },
            2 => Self::Unavailable {
                reason: Spooled::unspool(input)?,
            },
            3 => Self::NotRecorded {
                mode: Spooled::unspool(input)?,
            },
            _ => return Err(replay_spool::invalid("A spooled view of a kind it does not know")),
        })
    }
}

impl Placeholder for TableView {
    fn placeholder() -> Self {
        Self::Unavailable {
            reason: String::new(),
        }
    }
}

impl Spooled for SharedView {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        self.0.spool(out)?;
        self.1.spool(out)?;
        self.2.spool(out)?;
        self.3.spool(out)?;
        self.4.spool(out)?;
        match self.5.as_deref() {
            Some(super::Access::Keys(keys)) => Some(Arc::clone(keys)).spool(out),
            _ => Option::<Arc<super::Keys>>::None.spool(out),
        }
    }

    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self(
            Arc::unspool(input)?,
            Spooled::unspool(input)?,
            Spooled::unspool(input)?,
            Spooled::unspool(input)?,
            Spooled::unspool(input)?,
            Option::<Arc<super::Keys>>::unspool(input)?.map(|keys| Arc::new(super::Access::Keys(keys))),
        ))
    }
}

impl Spooled for ViewIdentity {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        self.0.spool(out)?;
        self.1.spool(out)?;
        self.2.spool(out)
    }

    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self(
            Weak::unspool(input)?,
            Spooled::unspool(input)?,
            Spooled::unspool(input)?,
        ))
    }
}

impl Spooled for ChangeFacts {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        self.previous.spool(out)?;
        self.changed.spool(out)?;
        self.deleted.spool(out)?;
        self.fields.spool(out)?;
        self.membership.spool(out)
    }

    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self {
            previous: Spooled::unspool(input)?,
            changed: Spooled::unspool(input)?,
            deleted: Spooled::unspool(input)?,
            fields: Spooled::unspool(input)?,
            membership: Spooled::unspool(input)?,
        })
    }
}

impl Spooled for FieldStamp {
    fn spool(&self, out: &mut Writer<'_>) -> io::Result<()> {
        self.epoch.spool(out)?;
        self.fields.spool(out)?;
        self.membership.spool(out)
    }

    fn unspool(input: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self {
            epoch: Spooled::unspool(input)?,
            fields: Spooled::unspool(input)?,
            membership: Spooled::unspool(input)?,
        })
    }
}
