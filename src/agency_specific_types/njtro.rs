use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrainDepartureScreenItem {
    #[serde(rename="background")]
    pub background_color: String,
    #[serde(rename="color")]
    pub foreground_color: String,
    #[serde(rename="departureDate")]
    pub departure_date: String,
    pub destination: String,
    #[serde(rename="inlineMessage")]
    pub inline_message: String,
    pub line: String,
    #[serde(rename="lineAbbreviation")]
    pub line_abbreviation: String,
    pub status: String,
    // pub stops: Option<Vec<TrainDepartureScreenStop>>
    // it actually is always null, so we removed it
    pub track: String,
    #[serde(rename="train_id")]
    pub train_num: String,
    pub capacity: Option<TrainDepartureScreenItemCapacity>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrainDepartureScreenItemCapacity {
    pub sections: Option<Vec<TrainDepartureScreenItemCapacitySection>>
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrainDepartureScreenItemCapacitySection {
    pub cars: Option<Vec<TrainDepartureScreenItemCapacitySectionCar>>
    pub position: TrainDepartureScreenItemCapacitySectionPosition,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum TrainDepartureScreenItemCapacitySectionPosition {
    Front,
    Middle,
    Back,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrainDepartureScreenItemCapacitySectionCar {
    pub color: Option<String>,
    pub number: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TrainScheduleStation {
    pub title: Option<String>,
    #[serde(rename="pentaStationID")]
    pub penta_station_id: Option<String>,
    pub accessible: Option<bool>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetTrainScheduleStationsRailForDVResponse {
    pub data: GetTrainScheduleStationsRailForDVResponseInner,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GetTrainScheduleStationsRailForDVResponseInner {
    #[serde(rename="getTrainScheduleStationsRailForDV")]
    pub data: Vec<TrainScheduleStation>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeparturesResponse {
    pub data: Map<String, DeparturesResponseInner>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeparturesResponseInner {
    pub items: Vec<TrainDepartureScreenItem>,
}
