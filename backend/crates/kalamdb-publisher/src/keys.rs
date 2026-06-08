use kalamdb_commons::models::{ConsumerGroupId, TopicId};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TopicPartitionKey {
    pub topic_id: TopicId,
    pub partition_id: u32,
}

impl TopicPartitionKey {
    #[inline]
    pub fn new(topic_id: &TopicId, partition_id: u32) -> Self {
        Self {
            topic_id: topic_id.clone(),
            partition_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct GroupPartitionKey {
    pub topic_id: TopicId,
    pub group_id: ConsumerGroupId,
    pub partition_id: u32,
}

impl GroupPartitionKey {
    #[inline]
    pub fn new(topic_id: &TopicId, group_id: &ConsumerGroupId, partition_id: u32) -> Self {
        Self {
            topic_id: topic_id.clone(),
            group_id: group_id.clone(),
            partition_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ConsumerGroupKey {
    pub topic_id: TopicId,
    pub group_id: ConsumerGroupId,
}

impl ConsumerGroupKey {
    #[inline]
    pub fn new(topic_id: &TopicId, group_id: &ConsumerGroupId) -> Self {
        Self {
            topic_id: topic_id.clone(),
            group_id: group_id.clone(),
        }
    }
}
