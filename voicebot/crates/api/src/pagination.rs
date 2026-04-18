use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

impl PaginationParams {
    pub fn offset(&self) -> i64 {
        let page = self.page.unwrap_or(1).max(1);
        let limit = self.limit_clamped();
        (page - 1) * limit
    }

    pub fn limit_clamped(&self) -> i64 {
        self.limit.unwrap_or(20).clamp(1, 100)
    }
}

#[derive(Debug, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub limit: i64,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, total: i64, params: &PaginationParams) -> Self {
        Self {
            items,
            total,
            page: params.page.unwrap_or(1).max(1),
            limit: params.limit_clamped(),
        }
    }
}
