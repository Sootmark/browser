//! How the browser came to a page: Chromium's page transition (a core type
//! in the low byte, qualifier bits above it) and Firefox's visit type;
//! Internet Explorer's WebCache doesn't record it.

use std::fmt;

/// How a visit began, in its browser's own terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transition {
    /// Chromium's `visits.transition`.
    Chromium(PageTransition),
    /// Firefox's `moz_historyvisits.visit_type`.
    Firefox(VisitType),
    /// Not recorded (Internet Explorer and legacy Edge's WebCache), or lost
    /// from a recovered visit's record.
    NotRecorded,
}

impl fmt::Display for Transition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Chromium(transition) => transition.fmt(f),
            Self::Firefox(visit_type) => visit_type.fmt(f),
            Self::NotRecorded => Ok(()),
        }
    }
}

/// Chromium's page transition, as `ui/base/page_transition_types.h`
/// defines it: the core type in the low byte, qualifiers in the bits
/// above. Displayed as the core type then each qualifier, `|`-separated
/// (`LINK|CHAIN_START|CHAIN_END`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageTransition(pub u32);

impl PageTransition {
    const CORE_MASK: u32 = 0xFF;

    /// The core type: what the user did.
    #[must_use]
    pub fn core(self) -> CoreTransition {
        CoreTransition::from_raw((self.0 & Self::CORE_MASK) as u8)
    }

    /// The qualifiers set, in bit order.
    pub fn qualifiers(self) -> impl Iterator<Item = Qualifier> {
        Qualifier::ALL
            .into_iter()
            .filter(move |q| self.0 & q.bit() != 0)
    }

    /// Qualifier bits set that Chromium doesn't define.
    #[must_use]
    pub fn unknown_qualifier_bits(self) -> u32 {
        let known = Qualifier::ALL
            .iter()
            .fold(Self::CORE_MASK, |m, q| m | q.bit());
        self.0 & !known
    }
}

impl fmt::Display for PageTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.core())?;
        for qualifier in self.qualifiers() {
            write!(f, "|{}", qualifier.name())?;
        }
        match self.unknown_qualifier_bits() {
            0 => Ok(()),
            bits => write!(f, "|{bits:#010x}"),
        }
    }
}

/// Chromium's core transition types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreTransition {
    /// `0`: a link was followed.
    Link,
    /// `1`: the URL was typed in the address bar.
    Typed,
    /// `2`: a bookmark, or a suggestion such as the new tab page's.
    AutoBookmark,
    /// `3`: a subframe loaded automatically (ads, embeds).
    AutoSubframe,
    /// `4`: a subframe navigated by the user.
    ManualSubframe,
    /// `5`: an address bar suggestion that wasn't a URL (a search).
    Generated,
    /// `6`: a top-level load not started by the user (command line, a
    /// start page).
    AutoToplevel,
    /// `7`: a form was submitted.
    FormSubmit,
    /// `8`: a reload, or a session restore.
    Reload,
    /// `9`: a keyword search other than the default engine.
    Keyword,
    /// `10`: the visit to the search engine's page a keyword search made.
    KeywordGenerated,
    /// Any other value, as written.
    Other(u8),
}

impl CoreTransition {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Link,
            1 => Self::Typed,
            2 => Self::AutoBookmark,
            3 => Self::AutoSubframe,
            4 => Self::ManualSubframe,
            5 => Self::Generated,
            6 => Self::AutoToplevel,
            7 => Self::FormSubmit,
            8 => Self::Reload,
            9 => Self::Keyword,
            10 => Self::KeywordGenerated,
            other => Self::Other(other),
        }
    }
}

impl fmt::Display for CoreTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Link => "LINK",
            Self::Typed => "TYPED",
            Self::AutoBookmark => "AUTO_BOOKMARK",
            Self::AutoSubframe => "AUTO_SUBFRAME",
            Self::ManualSubframe => "MANUAL_SUBFRAME",
            Self::Generated => "GENERATED",
            Self::AutoToplevel => "AUTO_TOPLEVEL",
            Self::FormSubmit => "FORM_SUBMIT",
            Self::Reload => "RELOAD",
            Self::Keyword => "KEYWORD",
            Self::KeywordGenerated => "KEYWORD_GENERATED",
            Self::Other(value) => return write!(f, "{value}"),
        };
        f.write_str(name)
    }
}

/// Chromium's transition qualifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Qualifier {
    /// `0x00800000`: a supervised user was blocked from the page.
    Blocked,
    /// `0x01000000`: the Back or Forward button.
    ForwardBack,
    /// `0x02000000`: started from the address bar.
    FromAddressBar,
    /// `0x04000000`: the home page.
    HomePage,
    /// `0x08000000`: started by another application.
    FromApi,
    /// `0x10000000`: the first page of a redirect chain.
    ChainStart,
    /// `0x20000000`: the last page of a redirect chain.
    ChainEnd,
    /// `0x40000000`: reached by a script or meta refresh redirect.
    ClientRedirect,
    /// `0x80000000`: reached by an HTTP redirect.
    ServerRedirect,
}

impl Qualifier {
    /// Every qualifier, in bit order.
    pub const ALL: [Self; 9] = [
        Self::Blocked,
        Self::ForwardBack,
        Self::FromAddressBar,
        Self::HomePage,
        Self::FromApi,
        Self::ChainStart,
        Self::ChainEnd,
        Self::ClientRedirect,
        Self::ServerRedirect,
    ];

    /// Its bit.
    #[must_use]
    pub const fn bit(self) -> u32 {
        match self {
            Self::Blocked => 0x0080_0000,
            Self::ForwardBack => 0x0100_0000,
            Self::FromAddressBar => 0x0200_0000,
            Self::HomePage => 0x0400_0000,
            Self::FromApi => 0x0800_0000,
            Self::ChainStart => 0x1000_0000,
            Self::ChainEnd => 0x2000_0000,
            Self::ClientRedirect => 0x4000_0000,
            Self::ServerRedirect => 0x8000_0000,
        }
    }

    /// Its name in Chromium, without the `PAGE_TRANSITION_` prefix.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Blocked => "BLOCKED",
            Self::ForwardBack => "FORWARD_BACK",
            Self::FromAddressBar => "FROM_ADDRESS_BAR",
            Self::HomePage => "HOME_PAGE",
            Self::FromApi => "FROM_API",
            Self::ChainStart => "CHAIN_START",
            Self::ChainEnd => "CHAIN_END",
            Self::ClientRedirect => "CLIENT_REDIRECT",
            Self::ServerRedirect => "SERVER_REDIRECT",
        }
    }
}

/// Firefox's visit types, `nsINavHistoryService`'s `TRANSITION_*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitType {
    /// `1`: a link was followed.
    Link,
    /// `2`: the URL was typed, or picked from the address bar's
    /// suggestions.
    Typed,
    /// `3`: a bookmark was opened.
    Bookmark,
    /// `4`: content embedded in a page (frames loaded with it).
    Embed,
    /// `5`: reached by a permanent redirect.
    RedirectPermanent,
    /// `6`: reached by a temporary redirect.
    RedirectTemporary,
    /// `7`: a download.
    Download,
    /// `8`: a link followed inside a frame.
    FramedLink,
    /// `9`: a reload.
    Reload,
    /// Any other value, as written.
    Other(i64),
}

impl VisitType {
    /// Each type and its value.
    const CODES: [(i64, Self); 9] = [
        (1, Self::Link),
        (2, Self::Typed),
        (3, Self::Bookmark),
        (4, Self::Embed),
        (5, Self::RedirectPermanent),
        (6, Self::RedirectTemporary),
        (7, Self::Download),
        (8, Self::FramedLink),
        (9, Self::Reload),
    ];

    pub(crate) fn from_raw(value: i64) -> Self {
        Self::CODES
            .iter()
            .find(|&&(code, _)| code == value)
            .map_or(Self::Other(value), |&(_, visit_type)| visit_type)
    }

    /// The value as stored.
    #[must_use]
    pub fn raw(self) -> i64 {
        match self {
            Self::Other(value) => value,
            known => Self::CODES
                .iter()
                .find(|&&(_, visit_type)| visit_type == known)
                .map_or(0, |&(code, _)| code),
        }
    }
}

impl fmt::Display for VisitType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Link => "LINK",
            Self::Typed => "TYPED",
            Self::Bookmark => "BOOKMARK",
            Self::Embed => "EMBED",
            Self::RedirectPermanent => "REDIRECT_PERMANENT",
            Self::RedirectTemporary => "REDIRECT_TEMPORARY",
            Self::Download => "DOWNLOAD",
            Self::FramedLink => "FRAMED_LINK",
            Self::Reload => "RELOAD",
            Self::Other(value) => return write!(f, "{value}"),
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromium_transitions_split_into_core_and_qualifiers() {
        // As stored in plaso's History file: a link alone in its chain, and
        // a subframe reached by an HTTP redirect.
        assert_eq!(
            PageTransition(805_306_368).to_string(),
            "LINK|CHAIN_START|CHAIN_END"
        );
        assert_eq!(
            PageTransition(2_684_354_563).to_string(),
            "AUTO_SUBFRAME|CHAIN_END|SERVER_REDIRECT"
        );
        assert_eq!(PageTransition(0x0040_0011).to_string(), "17|0x00400000");
        assert_eq!(PageTransition(1).core(), CoreTransition::Typed);
    }

    #[test]
    fn firefox_visit_types() {
        assert_eq!(VisitType::from_raw(2), VisitType::Typed);
        assert_eq!(VisitType::from_raw(0).to_string(), "0");
        assert_eq!(VisitType::from_raw(5).to_string(), "REDIRECT_PERMANENT");
    }
}
