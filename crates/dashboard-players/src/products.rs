pub mod player_apm;
pub mod campaign_clock;

dashboard_base::reply_warnings!(
    player_apm::ApmReply,
    campaign_clock::CampaignClockReply,
);
