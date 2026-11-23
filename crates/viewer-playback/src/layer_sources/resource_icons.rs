use viewer_layers_political::resource_icons::Source;
use crate::document::Content;
use viewer_runtime::resource_origins;

impl Source for Content {
    fn origins(&self) -> Option<&resource_origins::Content> {
        self.resource_icons.as_ref().ok().map(|icons| &icons.origins)
    }

    fn localisation(&self) -> &content_reader_localisation::localisation::Localisation {
        &self.names.localisation
    }
}
