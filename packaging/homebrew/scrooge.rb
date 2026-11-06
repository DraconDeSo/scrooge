cask "scrooge" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.0"
  sha256 arm:   "0000000000000000000000000000000000000000000000000000000000000000",
         intel: "0000000000000000000000000000000000000000000000000000000000000000"

  url "https://github.com/DraconDeSo/scrooge/releases/download/v#{version}/Scrooge_#{version}_#{arch}.dmg"
  name "Scrooge"
  desc "Desktop monitor for AI coding agents"
  homepage "https://github.com/DraconDeSo/scrooge"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :big_sur"

  app "Scrooge.app"

  zap trash: [
    "~/Library/Application Support/scrooge",
    "~/Library/Preferences/dev.scrooge.widget.plist",
  ]
end
