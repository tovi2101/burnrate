cask "burnrate" do
  version "1.3.0"
  sha256 :no_check

  url "https://github.com/tovi2101/burnrate/releases/download/v#{version}/Burnrate_#{version}_universal.dmg"
  name "Burnrate"
  desc "Local usage limits for AI coding providers"
  homepage "https://github.com/tovi2101/burnrate"

  app "Burnrate.app"
end
