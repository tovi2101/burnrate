cask "burnrate" do
  version "1.3.0"
  sha256 "0382a715111611a9bf459e1e0fe68f66eed879dea8bca67227f7ebc31c5e22c1"

  url "https://github.com/tovi2101/burnrate/releases/download/v#{version}/Burnrate_#{version}_universal.dmg"
  name "Burnrate"
  desc "Local usage limits for AI coding providers"
  homepage "https://github.com/tovi2101/burnrate"

  app "Burnrate.app"
end
