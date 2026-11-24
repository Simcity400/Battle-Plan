use engine_sites::Rva;
#[derive(Clone, Copy, Debug)]
pub struct Site {
    pub name: &'static str,
    pub rva: Rva,
    pub call_target: Option<Rva>,
}
pub const FUEL_SENT_SITE: usize = 4;
pub const FUEL_LOST_SITE: usize = 29;
pub const CONTRACT_REMOVAL_SITE: usize = 30;
pub const RETIRED_REFERENCE_ID: u32 = 0xfffe;
pub const SITES: [Site; 31] = [
    Site {
        name: "agreement_amendment",
        rva: Rva(0x19f7c60),
        call_target: None,
    },
    Site {
        name: "outgoing_action_reset",
        rva: Rva(0x159f830),
        call_target: None,
    },
    Site {
        name: "incoming_action_reset",
        rva: Rva(0x159f710),
        call_target: None,
    },
    Site {
        name: "agreement_removal",
        rva: Rva(0x19f7780),
        call_target: None,
    },
    Site {
        name: "agreement_fuel_delivered_store",
        rva: Rva(0x19fad0e),
        call_target: None,
    },
    Site {
        name: "equipment_delivery",
        rva: Rva(0x19f8f45),
        call_target: Some(Rva(0x1092970)),
    },
    Site {
        name: "equipment_loss",
        rva: Rva(0x19faf4a),
        call_target: Some(Rva(0x1092970)),
    },
    Site {
        name: "inventory_reserve",
        rva: Rva(0x1a67f9e),
        call_target: Some(Rva(0x1092b70)),
    },
    Site {
        name: "inventory_release",
        rva: Rva(0x1a67e4b),
        call_target: Some(Rva(0x1098700)),
    },
    Site {
        name: "inventory_return",
        rva: Rva(0x1a67c3d),
        call_target: Some(Rva(0x1092970)),
    },
    Site {
        name: "inventory_commitment",
        rva: Rva(0x1a68021),
        call_target: Some(Rva(0x1098680)),
    },
    Site {
        name: "price_overrides",
        rva: Rva(0x1452890),
        call_target: None,
    },
    Site {
        name: "subsidy_grant",
        rva: Rva(0x14527f0),
        call_target: None,
    },
    Site {
        name: "subsidy_use",
        rva: Rva(0x1452a50),
        call_target: None,
    },
    Site {
        name: "subsidy_return",
        rva: Rva(0x14526e0),
        call_target: None,
    },
    Site {
        name: "automation_options",
        rva: Rva(0x1bae1c0),
        call_target: None,
    },
    Site {
        name: "once_schedule_subtraction",
        rva: Rva(0x19f8bfd),
        call_target: Some(Rva(0x1098680)),
    },
    Site {
        name: "purchase_contract_registration",
        rva: Rva(0x1a66a70),
        call_target: None,
    },
    Site {
        name: "purchase_request_creation",
        rva: Rva(0x1a671f0),
        call_target: None,
    },
    Site {
        name: "purchase_request_removal",
        rva: Rva(0xe5b990),
        call_target: None,
    },
    Site {
        name: "purchase_contract_destruction",
        rva: Rva(0xe60420),
        call_target: None,
    },
    Site {
        name: "purchase_delivery",
        rva: Rva(0x1a65cc0),
        call_target: None,
    },
    Site {
        name: "purchase_delivered_items",
        rva: Rva(0x1a65dd6),
        call_target: Some(Rva(0x14535d0)),
    },
    Site {
        name: "purchase_payment",
        rva: Rva(0x1a65e06),
        call_target: Some(Rva(0x14f63e0)),
    },
    Site {
        name: "purchase_daily_delivery",
        rva: Rva(0x1a65be0),
        call_target: None,
    },
    Site {
        name: "route_equipment_rebuild",
        rva: Rva(0x1a66610),
        call_target: None,
    },
    Site {
        name: "route_equipment_generate",
        rva: Rva(0x1a666bc),
        call_target: Some(Rva(0x1455000)),
    },
    Site {
        name: "route_equipment_subtract",
        rva: Rva(0x1a666e1),
        call_target: Some(Rva(0x1098680)),
    },
    Site {
        name: "market_generated_item",
        rva: Rva(0x1455155),
        call_target: Some(Rva(0x1092b70)),
    },
    Site {
        name: "agreement_fuel_lost_store",
        rva: Rva(0x19fae8a),
        call_target: None,
    },
    Site {
        name: "purchase_contract_removal",
        rva: Rva(0x1a66b80),
        call_target: None,
    },
];
