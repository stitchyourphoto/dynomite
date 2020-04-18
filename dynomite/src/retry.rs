//! Retry functionality
//!
//! Specifically this implementation focuses on honoring [these documented DynamoDB retryable errors](https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/Programming.Errors.html#Programming.Errors.MessagesAndCodes)
//! on top AWS's general recommendations of for [retrying API requests](https://docs.aws.amazon.com/general/latest/gr/api-retries.html).
//!
//! # examples
//! ```rust,no_run
//!  use dynomite::{Retries, retry::Policy};
//!  use dynomite::dynamodb::{DynamoDb, DynamoDbClient};
//!
//!  let client =
//!     DynamoDbClient::new(Default::default())
//!         .with_retries(Policy::default());
//!
//!  // any client operation will now be retried when
//!  // appropriate
//!  let tables = client.list_tables(Default::default());
//! ```
//!
use crate::dynamodb::*;
use futures_backoff::{Condition, Strategy};
use log::debug;
#[cfg(feature = "default")]
use rusoto_core_default::RusotoError;
#[cfg(feature = "rustls")]
use rusoto_core_rustls::RusotoError;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

/// Preconfigured retry policies for failable operations
///
/// A `Default` impl of retrying 5 times with an exponential back-off of 100 milliseconds
#[derive(Clone, PartialEq, Debug)]
pub enum Policy {
    /// Limited number of times to retry
    Limit(usize),
    /// Limited number of times to retry with fixed pause between retries
    Pause(usize, Duration),
    /// Limited number of times to retry with an exponential pause between retries
    Exponential(usize, Duration),
}

impl Default for Policy {
    fn default() -> Self {
        Policy::Exponential(5, Duration::from_millis(100))
    }
}

impl Into<Strategy> for Policy {
    fn into(self) -> Strategy {
        match self {
            Policy::Limit(times) => Strategy::default()
                .with_max_retries(times)
                .with_jitter(true),
            Policy::Pause(times, duration) => Strategy::fixed(duration)
                .with_max_retries(times)
                .with_jitter(true),
            Policy::Exponential(times, duration) => Strategy::exponential(duration)
                .with_max_retries(times)
                .with_jitter(true),
        }
    }
}

/// Predicate trait that determines if an impl type is retryable
trait Retry {
    /// Return true if type is retryable
    fn retryable(&self) -> bool;
}

struct Counter(u16);

impl<R> Condition<RusotoError<R>> for Counter
where
    R: Retry,
{
    fn should_retry(
        &mut self,
        error: &RusotoError<R>,
    ) -> bool {
        debug!("retrying operation {}", self.0);
        if let Some(value) = self.0.checked_add(1) {
            self.0 = value;
        }
        match error {
            RusotoError::Service(e) => e.retryable(),
            _ => false,
        }
    }
}

// wrapper so we only pay for one arc
struct Inner<D> {
    client: D,
    strategy: Strategy,
}

/// A type which implements `DynamoDb` and retries all operations
/// that are retryable
#[derive(Clone)]
pub struct RetryingDynamoDb<D> {
    inner: Arc<Inner<D>>,
}

/// An interface for adapting a `DynamoDb` impl
/// to a `RetryingDynamoDb` impl
pub trait Retries<D>
where
    D: DynamoDb + 'static,
{
    /// Consumes a `DynamoDb` impl and produces
    /// a `DynamoDb` which retries its operations when appropriate
    fn with_retries(
        self,
        policy: Policy,
    ) -> RetryingDynamoDb<D>;
}

impl<D> Retries<D> for D
where
    D: DynamoDb + 'static,
{
    fn with_retries(
        self,
        policy: Policy,
    ) -> RetryingDynamoDb<D> {
        RetryingDynamoDb::new(self, policy)
    }
}

impl<D> RetryingDynamoDb<D>
where
    D: DynamoDb + 'static,
{
    /// Return a new instance with a configured retry policy
    pub fn new(
        client: D,
        policy: Policy,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                strategy: policy.into(),
            }),
        }
    }

    /// Retry and operation based on this clients configured retry policy
    #[inline]
    fn retry<'life0, 'async_trait, F, T, R>(
        &'life0 self,
        operation: F,
    ) -> Pin<Box<dyn Future<Output = Result<T, RusotoError<R>>> + Send + 'async_trait>>
    where
        F: FnMut()
            -> Pin<Box<dyn Future<Output = Result<T, RusotoError<R>>> + Send + 'async_trait>>,
        R: Retry,
    {
        RusotoFuture::from_future(self.inner.strategy.retry_if(operation, Counter(0)))
    }
}

impl<D> DynamoDb for RetryingDynamoDb<D>
where
    D: DynamoDb + Sync + Send + 'static,
{
    fn batch_get_item<'life0, 'async_trait>(
        &'life0 self,
        input: BatchGetItemInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BatchGetItemOutput, RusotoError<BatchGetItemError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.batch_get_item(input.clone()))
    }

    fn batch_write_item<'life0, 'async_trait>(
        &'life0 self,
        input: BatchWriteItemInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BatchWriteItemOutput, RusotoError<BatchWriteItemError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.batch_write_item(input.clone()))
    }

    fn create_backup<'life0, 'async_trait>(
        &'life0 self,
        input: CreateBackupInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<CreateBackupOutput, RusotoError<CreateBackupError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.create_backup(input.clone()))
    }

    fn create_global_table<'life0, 'async_trait>(
        &'life0 self,
        input: CreateGlobalTableInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<CreateGlobalTableOutput, RusotoError<CreateGlobalTableError>>,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.create_global_table(input.clone()))
    }

    fn create_table<'life0, 'async_trait>(
        &'life0 self,
        input: CreateTableInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<CreateTableOutput, RusotoError<CreateTableError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.create_table(input.clone()))
    }

    fn delete_backup<'life0, 'async_trait>(
        &'life0 self,
        input: DeleteBackupInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<DeleteBackupOutput, RusotoError<DeleteBackupError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.delete_backup(input.clone()))
    }

    fn delete_item<'life0, 'async_trait>(
        &'life0 self,
        input: DeleteItemInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<DeleteItemOutput, RusotoError<DeleteItemError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.delete_item(input.clone()))
    }

    fn delete_table<'life0, 'async_trait>(
        &'life0 self,
        input: DeleteTableInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<DeleteTableOutput, RusotoError<DeleteTableError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.delete_table(input.clone()))
    }

    fn describe_backup<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeBackupInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<DescribeBackupOutput, RusotoError<DescribeBackupError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_backup(input.clone()))
    }

    fn describe_continuous_backups<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeContinuousBackupsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        DescribeContinuousBackupsOutput,
                        RusotoError<DescribeContinuousBackupsError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_continuous_backups(input.clone()))
    }

    fn describe_global_table<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeGlobalTableInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        DescribeGlobalTableOutput,
                        RusotoError<DescribeGlobalTableError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_global_table(input.clone()))
    }

    fn describe_global_table_settings<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeGlobalTableSettingsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        DescribeGlobalTableSettingsOutput,
                        RusotoError<DescribeGlobalTableSettingsError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_global_table_settings(input.clone()))
    }

    fn describe_limits<'life0, 'async_trait>(
        &'life0 self
    ) -> Pin<
        Box<
            dyn Future<Output = Result<DescribeLimitsOutput, RusotoError<DescribeLimitsError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_limits())
    }

    fn describe_table<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeTableInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<DescribeTableOutput, RusotoError<DescribeTableError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_table(input.clone()))
    }

    fn describe_time_to_live<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeTimeToLiveInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<DescribeTimeToLiveOutput, RusotoError<DescribeTimeToLiveError>>,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_time_to_live(input.clone()))
    }

    fn get_item<'life0, 'async_trait>(
        &'life0 self,
        input: GetItemInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<GetItemOutput, RusotoError<GetItemError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.get_item(input.clone()))
    }

    fn list_backups<'life0, 'async_trait>(
        &'life0 self,
        input: ListBackupsInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ListBackupsOutput, RusotoError<ListBackupsError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.list_backups(input.clone()))
    }

    fn list_global_tables<'life0, 'async_trait>(
        &'life0 self,
        input: ListGlobalTablesInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ListGlobalTablesOutput, RusotoError<ListGlobalTablesError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.list_global_tables(input.clone()))
    }

    fn list_tables<'life0, 'async_trait>(
        &'life0 self,
        input: ListTablesInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ListTablesOutput, RusotoError<ListTablesError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.list_tables(input.clone()))
    }

    fn list_tags_of_resource<'life0, 'async_trait>(
        &'life0 self,
        input: ListTagsOfResourceInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<ListTagsOfResourceOutput, RusotoError<ListTagsOfResourceError>>,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.list_tags_of_resource(input.clone()))
    }

    fn put_item<'life0, 'async_trait>(
        &'life0 self,
        input: PutItemInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<PutItemOutput, RusotoError<PutItemError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.put_item(input.clone()))
    }

    fn query<'life0, 'async_trait>(
        &'life0 self,
        input: QueryInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<QueryOutput, RusotoError<QueryError>>> + Send + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.query(input.clone()))
    }

    fn restore_table_from_backup<'life0, 'async_trait>(
        &'life0 self,
        input: RestoreTableFromBackupInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        RestoreTableFromBackupOutput,
                        RusotoError<RestoreTableFromBackupError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.restore_table_from_backup(input.clone()))
    }

    fn restore_table_to_point_in_time<'life0, 'async_trait>(
        &'life0 self,
        input: RestoreTableToPointInTimeInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        RestoreTableToPointInTimeOutput,
                        RusotoError<RestoreTableToPointInTimeError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.restore_table_to_point_in_time(input.clone()))
    }

    fn scan<'life0, 'async_trait>(
        &'life0 self,
        input: ScanInput,
    ) -> Pin<
        Box<dyn Future<Output = Result<ScanOutput, RusotoError<ScanError>>> + Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.scan(input.clone()))
    }

    fn tag_resource<'life0, 'async_trait>(
        &'life0 self,
        input: TagResourceInput,
    ) -> Pin<
        Box<dyn Future<Output = Result<(), RusotoError<TagResourceError>>> + Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.tag_resource(input.clone()))
    }

    fn untag_resource<'life0, 'async_trait>(
        &'life0 self,
        input: UntagResourceInput,
    ) -> Pin<
        Box<dyn Future<Output = Result<(), RusotoError<UntagResourceError>>> + Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.untag_resource(input.clone()))
    }

    fn update_continuous_backups<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateContinuousBackupsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        UpdateContinuousBackupsOutput,
                        RusotoError<UpdateContinuousBackupsError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.update_continuous_backups(input.clone()))
    }

    fn update_global_table<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateGlobalTableInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<UpdateGlobalTableOutput, RusotoError<UpdateGlobalTableError>>,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.update_global_table(input.clone()))
    }

    fn update_global_table_settings<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateGlobalTableSettingsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        UpdateGlobalTableSettingsOutput,
                        RusotoError<UpdateGlobalTableSettingsError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.update_global_table_settings(input.clone()))
    }

    fn update_item<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateItemInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<UpdateItemOutput, RusotoError<UpdateItemError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.update_item(input.clone()))
    }

    fn update_table<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateTableInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<UpdateTableOutput, RusotoError<UpdateTableError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.update_table(input.clone()))
    }

    fn update_time_to_live<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateTimeToLiveInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<UpdateTimeToLiveOutput, RusotoError<UpdateTimeToLiveError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.update_time_to_live(input.clone()))
    }

    fn describe_endpoints<'life0, 'async_trait>(
        &'life0 self
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<DescribeEndpointsResponse, RusotoError<DescribeEndpointsError>>,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        // no apparent retryable errors
        self.inner.client.describe_endpoints()
    }

    fn transact_get_items<'life0, 'async_trait>(
        &'life0 self,
        input: TransactGetItemsInput,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TransactGetItemsOutput, RusotoError<TransactGetItemsError>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.transact_get_items(input.clone()))
    }

    fn transact_write_items<'life0, 'async_trait>(
        &'life0 self,
        input: TransactWriteItemsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<TransactWriteItemsOutput, RusotoError<TransactWriteItemsError>>,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.transact_write_items(input.clone()))
    }

    fn describe_contributor_insights<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeContributorInsightsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        DescribeContributorInsightsOutput,
                        RusotoError<DescribeContributorInsightsError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.describe_contributor_insights(input.clone()))
    }

    fn describe_table_replica_auto_scaling<'life0, 'async_trait>(
        &'life0 self,
        input: DescribeTableReplicaAutoScalingInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        DescribeTableReplicaAutoScalingOutput,
                        RusotoError<DescribeTableReplicaAutoScalingError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || {
            inner
                .client
                .describe_table_replica_auto_scaling(input.clone())
        })
    }

    fn list_contributor_insights<'life0, 'async_trait>(
        &'life0 self,
        input: ListContributorInsightsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        ListContributorInsightsOutput,
                        RusotoError<ListContributorInsightsError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        unimplemented!()
    }

    fn update_contributor_insights<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateContributorInsightsInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        UpdateContributorInsightsOutput,
                        RusotoError<UpdateContributorInsightsError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || inner.client.update_contributor_insights(input.clone()))
    }

    fn update_table_replica_auto_scaling<'life0, 'async_trait>(
        &'life0 self,
        input: UpdateTableReplicaAutoScalingInput,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        UpdateTableReplicaAutoScalingOutput,
                        RusotoError<UpdateTableReplicaAutoScalingError>,
                    >,
                > + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let inner = self.inner.clone();
        self.retry(move || {
            inner
                .client
                .update_table_replica_auto_scaling(input.clone())
        })
    }
}

/// retry impl for Service error types
macro_rules! retry {
    ($e:ty, $($p: pat)+) => {
        impl Retry for $e {
            fn retryable(&self) -> bool {
                // we allow unreachable_patterns because
                // _ => false because in some cases
                // all variants are retryable
                // in other cases, only a subset, hence
                // this type matching
                #[allow(unreachable_patterns)]
                match self {
                   $($p)|+ => true,
                    _ => false
                }
            }
        }
    }
}

retry!(
    BatchGetItemError,
    BatchGetItemError::InternalServerError(_) BatchGetItemError::ProvisionedThroughputExceeded(_)
);

retry!(
    BatchWriteItemError,
    BatchWriteItemError::InternalServerError(_) BatchWriteItemError::ProvisionedThroughputExceeded(_)
);

retry!(
    CreateBackupError,
    CreateBackupError::InternalServerError(_) CreateBackupError::LimitExceeded(_)
);

retry!(
    CreateGlobalTableError,
    CreateGlobalTableError::InternalServerError(_) CreateGlobalTableError::LimitExceeded(_)
);

retry!(
    CreateTableError,
    CreateTableError::InternalServerError(_) CreateTableError::LimitExceeded(_)
);

retry!(
    DeleteBackupError,
    DeleteBackupError::InternalServerError(_) DeleteBackupError::LimitExceeded(_)
);

retry!(
    DeleteItemError,
    DeleteItemError::InternalServerError(_) DeleteItemError::ProvisionedThroughputExceeded(_)
);

retry!(
    DeleteTableError,
    DeleteTableError::InternalServerError(_) DeleteTableError::LimitExceeded(_)
);

retry!(
    DescribeBackupError,
    DescribeBackupError::InternalServerError(_)
);

retry!(
    DescribeContinuousBackupsError,
    DescribeContinuousBackupsError::InternalServerError(_)
);

retry!(
    DescribeGlobalTableError,
    DescribeGlobalTableError::InternalServerError(_)
);

retry!(
    DescribeGlobalTableSettingsError,
    DescribeGlobalTableSettingsError::InternalServerError(_)
);

retry!(
    DescribeLimitsError,
    DescribeLimitsError::InternalServerError(_)
);

retry!(
    DescribeTableError,
    DescribeTableError::InternalServerError(_)
);

retry!(
    GetItemError,
    GetItemError::InternalServerError(_) GetItemError::ProvisionedThroughputExceeded(_)
);

retry!(ListBackupsError, ListBackupsError::InternalServerError(_));

retry!(ListTablesError, ListTablesError::InternalServerError(_));

retry!(
    ListTagsOfResourceError,
    ListTagsOfResourceError::InternalServerError(_)
);

retry!(
    PutItemError,
    PutItemError::InternalServerError(_) PutItemError::ProvisionedThroughputExceeded(_)
);

retry!(
    QueryError,
    QueryError::InternalServerError(_) QueryError::ProvisionedThroughputExceeded(_)
);

retry!(
    RestoreTableFromBackupError,
    RestoreTableFromBackupError::InternalServerError(_)
);

retry!(
    RestoreTableToPointInTimeError,
    RestoreTableToPointInTimeError::InternalServerError(_)
);

retry!(
    ScanError,
    ScanError::InternalServerError(_) ScanError::ProvisionedThroughputExceeded(_)
);

retry!(
    TagResourceError,
    TagResourceError::InternalServerError(_) TagResourceError::LimitExceeded(_)
);

retry!(
    UntagResourceError,
    UntagResourceError::InternalServerError(_) UntagResourceError::LimitExceeded(_)
);

retry!(
    UpdateContinuousBackupsError,
    UpdateContinuousBackupsError::InternalServerError(_)
);

retry!(
    UpdateGlobalTableError,
    UpdateGlobalTableError::InternalServerError(_)
);

retry!(
    UpdateGlobalTableSettingsError,
    UpdateGlobalTableSettingsError::InternalServerError(_)
);

retry!(
    UpdateItemError,
    UpdateItemError::InternalServerError(_) UpdateItemError::ProvisionedThroughputExceeded(_)
);

retry!(
    UpdateTableError,
    UpdateTableError::InternalServerError(_) UpdateTableError::LimitExceeded(_)
);

retry!(
    UpdateTimeToLiveError,
    UpdateTimeToLiveError::InternalServerError(_) UpdateTimeToLiveError::LimitExceeded(_)
);

retry!(
    ListGlobalTablesError,
    ListGlobalTablesError::InternalServerError(_)
);

retry!(
    DescribeTimeToLiveError,
    DescribeTimeToLiveError::InternalServerError(_)
);

retry!(
    TransactGetItemsError,
    TransactGetItemsError::InternalServerError(_) TransactGetItemsError::ProvisionedThroughputExceeded(_)
);

retry!(
    TransactWriteItemsError,
    TransactWriteItemsError::InternalServerError(_) TransactWriteItemsError::ProvisionedThroughputExceeded(_)
);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_has_default() {
        assert_eq!(
            Policy::default(),
            Policy::Exponential(5, Duration::from_millis(100))
        );
    }

    #[test]
    fn policy_impl_into_for_strategy() {
        // no great way to assert partialeq on strategy
        // just just test that we can
        let _: Strategy = Policy::default().into();
    }
}
