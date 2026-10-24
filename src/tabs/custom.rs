//! Собственные вкладки: каталог виджетов Controls, Today и Stats и сетка на каждую вкладку.
//!
//! Все собственные вкладки делят один `TabSpec` с виджетами всех трёх «хозяев»; в раскладке вкладки
//! виджет называется `хозяин.ключ` (например `today.clock`). Рисует виджет и принимает его нажатия
//! вкладка-хозяин, поэтому состояние (таймер, список дел, громкость) у виджета общее на всех вкладках,
//! где он лежит.

use std::sync::OnceLock;

use super::layout::{Spec, TabSpec, HIDDEN};
use super::{controls, stats, today, CONTROLS, STATS, TODAY};

/// Размер сетки собственной вкладки в ячейках: 10 колонок, 5 рядов.
pub const COLS: u8 = 10;
pub const ROWS: u8 = 5;

/// Вкладка, которой принадлежит виджет каталога.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Controls,
    Today,
    Stats,
}

pub const OWNERS: [Owner; 3] = [Owner::Controls, Owner::Today, Owner::Stats];

impl Owner {
    /// Описание виджетов вкладки-хозяина.
    pub fn spec(self) -> &'static TabSpec {
        match self {
            Owner::Controls => &controls::SPEC,
            Owner::Today => &today::SPEC,
            Owner::Stats => &stats::SPEC,
        }
    }

    /// Номер вкладки-хозяина.
    pub fn tab(self) -> usize {
        match self {
            Owner::Controls => CONTROLS,
            Owner::Today => TODAY,
            Owner::Stats => STATS,
        }
    }

    /// Приставка ключа в раскладке.
    fn token(self) -> &'static str {
        match self {
            Owner::Controls => "controls",
            Owner::Today => "today",
            Owner::Stats => "stats",
        }
    }

    /// Название группы в каталоге (русский ключ `i18n`).
    pub fn title(self) -> &'static str {
        match self {
            Owner::Controls => "Управление",
            Owner::Today => "Сегодня",
            Owner::Stats => "Статистика",
        }
    }
}

/// Каталог: общее описание виджетов и для каждого — хозяин и номер у него.
pub struct Catalog {
    pub spec: &'static TabSpec,
    pub map: Vec<(Owner, usize)>,
}

impl Catalog {
    /// Номер виджета в каталоге по хозяину и номеру у него.
    pub fn index(&self, owner: Owner, idx: usize) -> Option<usize> {
        self.map.iter().position(|&m| m == (owner, idx))
    }

    /// Хозяева виджетов, которые стоят на вкладке (не в лотке).
    pub fn owners_on(&self, lay: &super::layout::Layout) -> Vec<Owner> {
        let mut out: Vec<Owner> = Vec::new();
        for e in lay.entries.iter().filter(|e| !e.hidden) {
            if let Some(&(o, _)) = self.map.get(e.idx) {
                if !out.contains(&o) {
                    out.push(o);
                }
            }
        }
        out
    }
}

/// Название группы каталога по ключу виджета в раскладке (`today.clock` → «Сегодня»).
pub fn group_title(key: &str) -> Option<&'static str> {
    let (prefix, _) = key.split_once('.')?;
    OWNERS.iter().find(|o| o.token() == prefix).map(|o| o.title())
}

/// Каталог виджетов; собирается один раз. Все виджеты по умолчанию лежат в лотке: вкладка пустая,
/// пока пользователь не добавит нужные.
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut specs: Vec<Spec> = Vec::new();
        let mut map = Vec::new();
        for owner in OWNERS {
            for (i, s) in owner.spec().specs.iter().enumerate() {
                let key: &'static str = Box::leak(format!("{}.{}", owner.token(), s.key).into_boxed_str());
                specs.push(Spec { key, title: s.title, sizes: s.sizes, size: s.size, col: 0, row: HIDDEN });
                map.push((owner, i));
            }
        }
        let spec: &'static TabSpec = Box::leak(Box::new(TabSpec {
            cols: COLS,
            rows: ROWS,
            min_cols: 2,
            min_rows: 1,
            specs: Box::leak(specs.into_boxed_slice()),
        }));
        Catalog { spec, map }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_holds_every_owner_widget_once() {
        let c = catalog();
        let total: usize = OWNERS.iter().map(|o| o.spec().specs.len()).sum();
        assert_eq!(c.spec.specs.len(), total);
        assert_eq!(c.map.len(), total);
        assert!(total <= 32, "маска недоступных виджетов — u32");
        let mut keys: Vec<&str> = c.spec.specs.iter().map(|s| s.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), total, "ключи каталога не повторяются");
        assert_eq!(c.index(Owner::Today, 0), Some(controls::SPEC.specs.len()));
    }

    #[test]
    fn new_tab_is_empty_and_every_widget_fits_the_grid() {
        let c = catalog();
        let lay = super::super::layout::Layout::default_for(c.spec);
        assert!(lay.entries.iter().all(|e| e.hidden));
        assert!(c.owners_on(&lay).is_empty());
        let spec = super::super::layout::resolve_full(c.spec, 8);
        for s in spec.specs {
            assert!(s.sizes.iter().any(|z| z.w <= COLS && z.h <= ROWS), "{} не помещается в сетку", s.key);
        }
    }
}
