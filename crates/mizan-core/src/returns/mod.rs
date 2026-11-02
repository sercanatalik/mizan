//! The returns engine: daily value series per account and for the portfolio,
//! time- and money-weighted returns, and benchmark comparison. Computed from
//! stored data only (user database plus the shared history cache).

pub mod calc;
pub mod inflation;
pub mod pricebook;
pub mod series;
pub mod view;

use mizan_import::{Code, Date};
use mizan_storage::{HistoryCache, StorageError, Store};

pub use inflation::{DailyIndex, IndexDay, Inflation};
pub use pricebook::{DatedPrice, PriceBook};
pub use series::{account_series, replay_account, AccountReplay, AccountSeries, DayValue};
pub use view::{
    returns_view, value_history, Benchmark, Figure, PeriodReturns, ReturnsView, ValueHistory,
};

use crate::valuation::base_currency;

/// Everything the engine needs, loaded once.
pub struct Inputs {
    pub base: Code,
    pub book: PriceBook,
    pub accounts: Vec<AccountReplay>,
    /// The base currency's inflation index, for real returns.
    pub inflation: Inflation,
}

impl Inputs {
    pub fn load(store: &Store, cache: Option<&HistoryCache>) -> Result<Self, StorageError> {
        let keys = store.key_map()?;
        let mut accounts = Vec::new();
        for a in store.list_accounts()? {
            let stored = store.all_activities(a.id)?;
            accounts.push(replay_account(a.id, &a.name, &stored, &keys));
        }
        let base = base_currency(store)?;
        Ok(Inputs {
            inflation: Inflation::load(cache, &base)?,
            book: PriceBook::load(store, cache)?,
            base,
            accounts,
        })
    }

    /// Daily series for every account up to `today`.
    pub fn series(&self, today: Date) -> Vec<AccountSeries> {
        self.accounts
            .iter()
            .map(|r| account_series(r, &self.book, &self.base, today))
            .collect()
    }
}
