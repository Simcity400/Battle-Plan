pub mod army;
pub mod army_losses;
pub mod forces;
pub mod divisions;
pub mod division_log;
pub mod training_queue;
pub mod land_battles;
pub mod land_terrain;
pub mod leader_name_runs;
pub mod leaders;
pub mod doctrines;
pub mod doctrine_mastery;

dashboard_base::reply_warnings!(
    doctrines::DoctrineReply,
);
