require 'fileutils'
require 'json'
require 'open-uri'
require 'benchmark'
require 'pathname'
require 'uri'
require 'rake/clean'

module OS
  def OS.windows?
    (/cygwin|mswin|mingw|bccwin|wince|emx/ =~ RUBY_PLATFORM) != nil
  end

  def OS.mac?
   (/darwin/ =~ RUBY_PLATFORM) != nil
  end

  def OS.unix?
    !OS.windows?
  end

  def OS.linux?
    OS.unix? and not OS.mac?
  end

  def OS.jruby?
    RUBY_ENGINE == 'jruby'
  end
end

NPM_RUN = "npm run --quiet"
NPM_INSTALL = "npm install --prefere-offline"
DIST_FOLDER = "application/electron/dist"
COMPILED_CLIENT_FOLDER = "application/client.core/dist/logviewer"
COMPILED_FOLDER = "application/electron/dist/compiled"
RELEASE_FOLDER = "application/electron/dist/release"
INCLUDED_PLUGINS_FOLDER = "application/electron/dist/compiled/plugins"
INCLUDED_APPS_FOLDER = "application/electron/dist/compiled/apps"
APP_PACKAGE_JSON = "application/electron/package.json"
SRC_HOST_IPC = "application/electron/src/controllers/electron.ipc.messages"
DEST_CLIENT_HOST_IPC = "application/client.core/src/app/environment/services/electron.ipc.messages"
SRC_PLUGIN_IPC = "application/electron/src/controllers/plugins.ipc.messages"
DEST_CLIENT_PLUGIN_IPC = "application/client.core/src/app/environment/services/plugins.ipc.messages"
DEST_PLUGINIPCLIG_PLUGIN_IPC = "application/node.libs/logviewer.plugin.ipc/src/ipc.messages"
SRC_CLIENT_NPM_LIBS = "application/client.libs/logviewer.client.components"
RIPGREP_URL = "https://github.com/BurntSushi/ripgrep/releases/download/11.0.2/ripgrep-11.0.2"
DESTS_CLIENT_NPM_LIBS = [
  "application/client.core/node_modules",
  "application/client.plugins/node_modules"
]
CLIENT_NPM_LIBS_NAMES = [
  "logviewer-client-containers",
  "logviewer-client-primitive",
  "logviewer-client-complex",
]
PLUGINS_SANDBOX = "application/sandbox"

directory DIST_FOLDER
directory COMPILED_FOLDER
directory RELEASE_FOLDER
directory INCLUDED_PLUGINS_FOLDER
directory INCLUDED_APPS_FOLDER

FOLDERS_TO_CLEAN = [DIST_FOLDER, COMPILED_FOLDER, RELEASE_FOLDER, INCLUDED_PLUGINS_FOLDER, INCLUDED_APPS_FOLDER]
CLEAN.include(FOLDERS_TO_CLEAN)

task :folders => [DIST_FOLDER, COMPILED_FOLDER, RELEASE_FOLDER, INCLUDED_PLUGINS_FOLDER, INCLUDED_APPS_FOLDER]

SRC_LAUNCHER = "application/apps/launcher/target/release/launcher"
RELEASE_PATH = "application/electron/dist/release/"

if OS.windows? == true
  TARGET_PLATFORM_NAME = "win64"
  TARGET_PLATFORM_ALIAS = "win"
elsif OS.mac? == true
  TARGET_PLATFORM_NAME = "darwin"
  TARGET_PLATFORM_ALIAS = "mac"
else
  TARGET_PLATFORM_NAME = "linux"
  TARGET_PLATFORM_ALIAS = "linux"
end

puts "Detected target platform is: #{TARGET_PLATFORM_NAME} / #{TARGET_PLATFORM_ALIAS}"

def compress_plugin(file, dest)
  case TARGET_PLATFORM_ALIAS
    when "mac"
      sh "tar -czf #{file} -C #{PLUGINS_SANDBOX} #{dest} "
    when "linux"
      sh "tar -czf #{file} -C #{PLUGINS_SANDBOX} #{dest} "
    when "win"
      sh "tar -czf #{file} -C #{PLUGINS_SANDBOX} #{dest} --force-local"
  end
end

def get_nodejs_platform()
  platform_tag = ""
  if OS.windows? == true
    platform_tag = "win32"
  elsif OS.mac? == true
    platform_tag = "darwin"
  else
    platform_tag = "linux"
  end
  return platform_tag
end

desc "start"
task :start do
  cd "application/electron" do
    sh "#{NPM_RUN} electron"
  end
end

desc "prepare"
task :prepare do
  puts "Installing npm libs, which is needed for installing / updateing process"
  sh "npm install typescript --global --prefere-offline"
end

desc "ripgrep delivery"
task :ripgrepdelivery => :folders do
  path = "temp"
  Dir.mkdir(path) unless File.exists?(path)
  case TARGET_PLATFORM_ALIAS
    when "mac"
      url = "#{RIPGREP_URL}-x86_64-apple-darwin.tar.gz"
    when "linux"
      url = "#{RIPGREP_URL}-x86_64-unknown-linux-musl.tar.gz"
    when "win"
      url = "#{RIPGREP_URL}-x86_64-pc-windows-msvc.zip"
  end
  file_name = URI(url).path.split('/').last
  unix_version_platform = File.basename(file_name, ".tar.gz")

  open("#{path}/#{file_name}", "wb") do |file|
    file << open(url).read
  end
  case TARGET_PLATFORM_ALIAS
    when "mac"
      cd path do
        sh "tar xvzf #{file_name}"
      end
      src = "#{path}/#{unix_version_platform}/rg"
      dest = "#{COMPILED_FOLDER}/apps/rg"
    when "linux"
      cd path do
        sh "tar xvzf #{file_name}"
      end
      src = "#{path}/#{unix_version_platform}/rg"
      dest = "#{COMPILED_FOLDER}/apps/rg"
    when "win"
      cd path do
        sh "unzip #{file_name}"
      end
      src = "#{path}/rg.exe"
      dest = "#{COMPILED_FOLDER}/apps/rg.exe"
  end
  rm(dest) unless !File.exists?(dest)
  cp(src, dest)
  rm_r(path) unless !File.exists?(path)
end

task :build_client_core do
  cd "application/client.core" do
    puts "Installing: core"
    sh NPM_INSTALL
    sh "npm uninstall logviewer.client.toolkit"
    sh "npm install logviewer.client.toolkit@latest --prefere-offline"
  end
end
task :build_client_components do
  cd "application/client.libs/logviewer.client.components" do
    puts "Installing: components"
    sh NPM_INSTALL
  end
end
task :build_client_plugins do
  cd "application/client.plugins" do
    puts "Installing: plugins env"
    sh NPM_INSTALL
    sh "npm uninstall logviewer.client.toolkit"
    sh "npm install logviewer.client.toolkit@latest --prefere-offline"
  end
end
task :build_electron => [:prepare_electron_build, :build_embedded_indexer, :finish_electron_build]
task :finish_electron_build do
  cd "application/electron" do
    sh "#{NPM_RUN} build-ts"
  end
end
task :prepare_electron_build do
  cd "application/electron" do
    sh NPM_INSTALL
  end
end

desc "install"
task :install => [:folders,
                  :build_client_core,
                  :build_client_components,
                  :build_electron,
                  :ipc,
                  :clientlibsbuild,
                  :clientlibsdelivery,
                  :clientbuild,
                  :apppackagedelivery,
]

desc "Developer task: update client"
task :dev_update_client do
  Rake::Task["ipc"].invoke
  Rake::Task["clientbuild"].invoke
end

desc "Developer task: update client"
task :dev_fullupdate_client do
  Rake::Task["clientlibsbuild"].invoke
  Rake::Task["clientlibsdelivery"].invoke
  Rake::Task["dev_update_client"].invoke
end

desc "Developer task: update client"
task :dev_fullupdate_client_run do
  Rake::Task["dev_fullupdate_client"].invoke
  cd "application/electron" do
    sh "#{NPM_RUN} electron"
  end
end

#Application should be built already to use this task
desc "Developer task: build launcher and delivery into package."
task :dev_build_delivery_apps do

  Rake::Task["buildlauncher"].invoke
  Rake::Task["buildupdater"].invoke


  case TARGET_PLATFORM_ALIAS
    when "mac"
      node_app_original = "#{RELEASE_PATH}mac/chipmunk.app/Contents/MacOS/chipmunk"
      launcher = SRC_LAUNCHER
    when "linux"
      node_app_original = "#{RELEASE_PATH}linux-unpacked/chipmunk"
      launcher = SRC_LAUNCHER
    when "win"
      node_app_original = "#{RELEASE_PATH}win-unpacked/chipmunk.exe"
      launcher = "#{SRC_LAUNCHER}.exe"
  end
  rm(node_app_original)
  cp(launcher, node_app_original)

end

desc "ipc"
task :ipc do
  puts "Delivery IPC definitions"
  $paths = [DEST_CLIENT_HOST_IPC, DEST_CLIENT_PLUGIN_IPC, DEST_PLUGINIPCLIG_PLUGIN_IPC];
  i = 0;
  while i < $paths.length
    path = $paths[i]
    rm_r(path) unless !File.exists?(path)
    i += 1
  end
  cp_r(SRC_HOST_IPC, DEST_CLIENT_HOST_IPC, :verbose => false)
  cp_r(SRC_PLUGIN_IPC, DEST_CLIENT_PLUGIN_IPC, :verbose => false)
  cp_r(SRC_PLUGIN_IPC, DEST_PLUGINIPCLIG_PLUGIN_IPC, :verbose => false)
end

desc "Building client libs"
task :clientlibsbuild do
  puts "Building client libs"
  cd SRC_CLIENT_NPM_LIBS do
    i = 0;
    while i < CLIENT_NPM_LIBS_NAMES.length
      lib = CLIENT_NPM_LIBS_NAMES[i]
      puts "Compiling client components library: #{lib}"
      sh "#{NPM_RUN} build:#{lib}"
      i += 1
    end
  end
end

desc "Delivery client libs"
task :clientlibsdelivery do
  puts "Delivery client libs"
  i = 0;
  while i < DESTS_CLIENT_NPM_LIBS.length
    dest = DESTS_CLIENT_NPM_LIBS[i]
    puts "Delivery libs into: #{dest}"
    if !File.exists?(dest)
      puts "NPM isn't installed in project #{File.dirname(dest)}. Installing..."
      cd File.dirname(dest) do
        sh NPM_INSTALL
      end
    end
    j = 0;
    while j < CLIENT_NPM_LIBS_NAMES.length
      lib = CLIENT_NPM_LIBS_NAMES[j]
      src = "#{SRC_CLIENT_NPM_LIBS}/dist/#{lib}"
      path = "#{dest}/#{lib}"
      puts src
      puts path
      rm_r(path) unless !File.exists?(path)
      cp_r(src, path, :verbose => false)
      j += 1
    end
    i += 1
  end
end

desc "Build client"
task :clientbuild do
  cd "application/client.core" do
    puts "Building client.core"
    sh "#{NPM_RUN} build"
  end
  puts "Delivery client.core"
  dest_client_path = "#{COMPILED_FOLDER}/client"
  rm_r(dest_client_path) unless !File.exists?(dest_client_path)
  cp_r(COMPILED_CLIENT_FOLDER, dest_client_path, :verbose => false)
end

desc "Add package.json to compiled app"
task :apppackagedelivery do
  cp_r(APP_PACKAGE_JSON, "#{COMPILED_FOLDER}/package.json", :verbose => false)
end

desc "install plugins"
task :plugins => [:folders, :pluginsstandalone, :pluginscomplex, :pluginsangular]

desc "Install standalone plugins"
task :pluginsstandalone do
  complex_plugins = ["row.parser.ascii"];
  i = 0
  while i < complex_plugins.length
    plugin = complex_plugins[i]
    puts "Installing plugin: #{plugin}"
    src = "application/client.plugins.standalone/#{plugin}"
    cd src do
      puts "Install plugin: #{plugin}"
      sh NPM_INSTALL
      sh "npm uninstall logviewer.client.toolkit"
      sh "npm install logviewer.client.toolkit@latest --prefere-offline"
      sh "#{NPM_RUN} build"
    end
    dest = "#{PLUGINS_SANDBOX}/#{plugin}"
    rm_r("#{dest}/render/dist") unless !File.exists?("#{dest}/render/dist")
    cp_r("#{src}/dist", "#{dest}/render/dist", :verbose => false)
    cp_r("#{src}/package.json", "#{dest}/render/package.json", :verbose => false)
    package_str = File.read("#{dest}/render/package.json")
    package = JSON.parse(package_str)
    arch = "#{INCLUDED_PLUGINS_FOLDER}/#{plugin}@#{package["version"]}-#{get_nodejs_platform()}.tgz"
    rm(arch) unless !File.exists?(arch)
    compress_plugin(arch, plugin)
    i += 1
  end
end

desc "Install complex plugins"
task :pluginscomplex do
  complex_plugins = ["dlt", "serial", "processes", "xterminal"];
  i = 0
  while i < complex_plugins.length
    plugin = complex_plugins[i]
    puts "Installing plugin: #{plugin}"
    cd "application/sandbox/#{plugin}/process" do
      sh NPM_INSTALL
      sh "npm install electron@4.0.3 electron-rebuild@^1.8.2 --prefere-offline"
      sh "./node_modules/.bin/electron-rebuild"
      sh "npm uninstall electron electron-rebuild"
      sh "#{NPM_RUN} build"
    end
    cd "application/client.plugins" do
      sh "#{NPM_RUN} build:#{plugin}"
    end
    src = "application/client.plugins/dist/#{plugin}"
    dest = "#{PLUGINS_SANDBOX}/#{plugin}"
    rm_r("#{dest}/render") unless !File.exists?("#{dest}/render")
    cp_r("#{src}", "#{dest}/render", :verbose => false)
    package_str = File.read("#{dest}/process/package.json")
    package = JSON.parse(package_str)
    arch = "#{INCLUDED_PLUGINS_FOLDER}/#{plugin}@#{package["version"]}-#{get_nodejs_platform()}.tgz"
    compress_plugin(arch, plugin)
    i += 1
  end
end

desc "Install render (angular) plugins"
task :pluginsangular do
  complex_plugins = ["dlt-render"];
  i = 0
  while i < complex_plugins.length
    plugin = complex_plugins[i]
    puts "Installing plugin: #{plugin}"
    cd "application/client.plugins" do
      sh "#{NPM_RUN} build:#{plugin}"
    end
    src = "application/client.plugins/dist/#{plugin}"
    dest = "#{PLUGINS_SANDBOX}/#{plugin}"
    rm_r("#{dest}/render") unless !File.exists?("#{dest}/render")
    cp_r("#{src}", "#{dest}/render", :verbose => false)
    package_str = File.read("#{dest}/render/package.json")
    package = JSON.parse(package_str)
    arch = "#{INCLUDED_PLUGINS_FOLDER}/#{plugin}@#{package["version"]}-#{get_nodejs_platform()}.tgz"
    compress_plugin(arch, plugin)
    i += 1
  end
end

desc "update plugin.ipc"
task :updatepluginipc do
  cd "application/sandbox/dlt/process" do
    puts "Update toolkits for: dlt plugin"
    sh "npm uninstall logviewer.plugin.ipc"
    sh "npm install logviewer.plugin.ipc@latest --prefere-offline"
  end
  cd "application/sandbox/serial/process" do
    puts "Update toolkits for: serial plugin"
    sh "npm uninstall logviewer.plugin.ipc"
    sh "npm install logviewer.plugin.ipc@latest --prefere-offline"
  end
  cd "application/sandbox/processes/process" do
    puts "Update toolkits for: xterminal pluginplugin"
    sh "npm uninstall logviewer.plugin.ipc"
    sh "npm install logviewer.plugin.ipc@latest --prefere-offline"
  end
  cd "application/sandbox/xterminal/process" do
    puts "Update toolkits for: xterminal plugin"
    sh "npm uninstall logviewer.plugin.ipc"
    sh "npm install logviewer.plugin.ipc@latest --prefere-offline"
  end
end

desc "build updater"
task :buildupdater => :folders do

  src_app_dir = "application/apps/updater/target/release/"
  app_file = "updater"

  if OS.windows? == true
    app_file = "updater.exe"
  end

  cd "application/apps/updater" do
    puts 'Build updater'
    sh "cargo build --release"
  end

  puts "Check old version of app: #{INCLUDED_APPS_FOLDER}/#{app_file}"
  rm("#{INCLUDED_APPS_FOLDER}/#{app_file}") unless !File.exists?("#{INCLUDED_APPS_FOLDER}/#{app_file}")
  puts "Updating app from: #{src_app_dir}#{app_file}"
  cp("#{src_app_dir}#{app_file}", "#{INCLUDED_APPS_FOLDER}/#{app_file}")

end

desc "build launcher"
task :buildlauncher => :folders do

  src_app_dir = "application/apps/launcher/target/release/"
  app_file = "launcher"

  if OS.windows? == true
    app_file = "launcher.exe"
  end

  cd "application/apps/launcher" do
    puts 'Build launcher'
    sh "cargo build --release"
  end

  puts "Check old version of app: #{INCLUDED_APPS_FOLDER}/#{app_file}"
  rm("#{INCLUDED_APPS_FOLDER}/#{app_file}") unless !File.exists?("#{INCLUDED_APPS_FOLDER}/#{app_file}")
  puts "Updating app from: #{src_app_dir}#{app_file}"
  cp("#{src_app_dir}#{app_file}", "#{INCLUDED_APPS_FOLDER}/#{app_file}")

end

desc "build indexer"
task :buildindexer do
  Rake::Task["folders"].invoke

  src_app_dir = "application/apps/indexer/target/release/"
  app_file_comp = "indexer_cli"
  app_file_release = "lvin"

  if OS.windows? == true
    app_file_comp = "indexer_cli.exe"
    app_file_release = "lvin.exe"
  end

  cd "application/apps/indexer" do
    puts 'Build indexer'
    sh "cargo build --release"
  end

  puts "Check old version of app: #{INCLUDED_APPS_FOLDER}/#{app_file_release}"
  rm("#{INCLUDED_APPS_FOLDER}/#{app_file_release}") unless !File.exists?("#{INCLUDED_APPS_FOLDER}/#{app_file_release}")
  puts "Updating app from: #{src_app_dir}#{app_file_comp}"
  cp("#{src_app_dir}#{app_file_comp}", "#{INCLUDED_APPS_FOLDER}/#{app_file_release}")

end

def fresh_folder(dest_folder)
  rm_r(dest_folder) unless !File.exists?(dest_folder)
  mkdir_p dest_folder
end

desc "build embedded indexer"
task :build_embedded_indexer do
  cd "application/apps/indexer-neon" do
    sh NPM_INSTALL
    sh "#{NPM_RUN} build"
  end
  src_folder = Pathname.new("application/apps/indexer-neon")
  dest_folder = Pathname.new("application/electron/node_modules/indexer-neon")
  puts "Delivery indexer from: #{src_folder} into #{dest_folder}"
  fresh_folder(dest_folder)

  Dir[src_folder.join "*"]
    .reject { |n| n.end_with? "node_modules" or n.end_with? "native" }
    .each do |s|
      cp_r(s, dest_folder, :verbose => false)
  end
  dest_native = dest_folder.join("native")
  dest_native_target = dest_native.join("target")
  fresh_folder(dest_native_target)
  ["Cargo.lock", "Cargo.toml", "artifacts.json", "build.rs", "index.node", "src"].each do |f|
    cp_r(src_folder.join("native").join(f), dest_native, :verbose => false)
  end
  cp_r(src_folder.join("native").join("target").join("release"), dest_native_target, :verbose => false)
end

desc "full update"
task :update => [:buildlauncher, :buildupdater, :buildindexer]

desc "create list of files and folder in release"
task :setlistofreleasefiles do
  puts "Prepare list of files/folders in release"
  case TARGET_PLATFORM_ALIAS
    when "mac"
      puts "No need to do it for mac"
      next
    when "linux"
      path = "#{RELEASE_FOLDER}/linux-unpacked"
    when "win"
      path = "#{RELEASE_FOLDER}/win-unpacked"
  end
  if !File.exists?(path)
    abort("No release found at #{path}")
  end
  destfile = "#{path}/.release"
  rm(destfile) unless !File.exists?(destfile)
  lines = ".release\n";
  Dir.foreach(path) {|entry|
    if entry != "." && entry != ".."
      lines = "#{lines}#{entry}\n"
    end
  }
  File.open(destfile, "a") do |line|
    line.puts lines
  end
end

desc "build"
task :build do

  rm_r(RELEASE_FOLDER) unless !File.exists?(RELEASE_FOLDER)
  Rake::Task["folders"].invoke

  cd "application/electron" do
    sh "#{NPM_RUN} build-ts"
    sh "./node_modules/.bin/build --#{TARGET_PLATFORM_ALIAS}"
  end


  case TARGET_PLATFORM_ALIAS
    when "mac"
      mv("#{RELEASE_PATH}mac/chipmunk.app/Contents/MacOS/chipmunk", "#{RELEASE_PATH}mac/chipmunk.app/Contents/MacOS/app")
      cp("#{SRC_LAUNCHER}", "#{RELEASE_PATH}mac/chipmunk.app/Contents/MacOS/chipmunk")
    when "linux"
      mv("#{RELEASE_PATH}linux-unpacked/chipmunk", "#{RELEASE_PATH}linux-unpacked/app")
      cp("#{SRC_LAUNCHER}", "#{RELEASE_PATH}linux-unpacked/chipmunk")
    when "win"
      mv("#{RELEASE_PATH}win-unpacked/chipmunk.exe", "#{RELEASE_PATH}win-unpacked/app.exe")
      cp("#{SRC_LAUNCHER}.exe", "#{RELEASE_PATH}win-unpacked/chipmunk.exe")
  end

end

desc "Prepare package to deploy on Github"
task :prepare_to_deploy do
  puts "===== prepare_to_deploy"
  time = Benchmark.measure do
    package_str = File.read(APP_PACKAGE_JSON)
    package = JSON.parse(package_str)
    puts "Detected version: #{package["version"]}"
    cd "application/electron/dist/release" do
      release_name = "chipmunk@#{package["version"]}-#{TARGET_PLATFORM_NAME}-portable"
      case TARGET_PLATFORM_ALIAS
        when "mac"
          cd "mac" do
            sh "tar -czf ../#{release_name}.tgz ./chipmunk.app"
          end
        when "linux"
          cd "#{TARGET_PLATFORM_ALIAS}-unpacked" do
            sh "tar -czf ../#{release_name}.tgz *"
          end
        when "win"
          cd "#{TARGET_PLATFORM_ALIAS}-unpacked" do
            sh "tar -czf ../#{release_name}.tgz ./* --force-local"
          end
      end
    end
  end
  puts "prepare_to_deploy took #{time}"
end

desc "Build the full build pipeline for a given platform"
task :full_pipeline => [:clean, :prepare, :install, :update, :plugins, :ripgrepdelivery, :build, :setlistofreleasefiles, :prepare_to_deploy]

$task_benchmarks = []

class Rake::Task
  def execute_with_benchmark(*args)
    puts "******* running task #{name}"
    bm = Benchmark.realtime { execute_without_benchmark(*args) }
    $task_benchmarks << [name, bm]
    puts ">>>>>>>    #{name} --> #{'%.1f' % bm} s"
  end

  alias_method :execute_without_benchmark, :execute
  alias_method :execute, :execute_with_benchmark
end

task :a do
  puts "a"
  sleep(0.6)
end
task :b => :a do
  puts "b"
  sleep(0.5)
end
task :c => :b do
  puts "c"
  sleep(0.1)
end

at_exit do
  total_time = $task_benchmarks.reduce(0) {|acc, x| acc + x[1]}
  $task_benchmarks
    .sort { |a, b| b[1] <=> a[1] }
    .each do |res|
    percentage = res[1]/total_time * 100
    if percentage.round > 0
      percentage_bar = ""
      percentage.round.times { percentage_bar += "|" }
      puts "#{percentage_bar} (#{'%.1f' % percentage} %) #{res[0]} ==> #{'%.1f' % res[1]}s"
    end
  end
  puts "total time was: #{'%.1f' % total_time}"
end
