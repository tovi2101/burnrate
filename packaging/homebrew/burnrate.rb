cask "burnrate" do
  version "1.0.0"
  sha256 "91911b7dd536aa4cdcbf062d538dd343cb504fc09c52450ab944459a7bceec62"

  url "https://github.com/tovi2101/burnrate/releases/download/v#{version}/Burnrate_#{version}_universal.dmg"
  name "Burnrate"
  desc "Local usage limits for AI coding providers"
  homepage "https://github.com/tovi2101/burnrate"

  app "Burnrate.app"
end
