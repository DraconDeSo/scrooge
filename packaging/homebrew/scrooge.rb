cask "scrooge" do
  arch arm: "aarch64", intel: "x64"

  version "0.1.1"
  sha256 arm:   "236cdaf8a3e5ab01a03d576875b96f392c5d17a3386d29a51e538ea9be07be8d",
         intel: "6cb8fc7d66a2252ee9936f8faf988f71b2301bfd0cc375448dd5796e874a5471"

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
