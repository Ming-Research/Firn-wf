# frozen_string_literal: true

# What Sidekiq's `rake test` has loaded when any case runs, for a process that
# runs one file: rake loads every file of test/ into one process before it
# runs a case, so a file may use what another loaded. Run alone, four files
# lacked these, and their cases failed on Redis 7.2.16 too:
#
# - Sidekiq::DeadSet, from `sidekiq/api` (retry_exhausted_test.rb)
# - Rack::Session, from `rack/session` (filtering_test.rb, profiling_test.rb)
# - Rails.application, the dummy application that the pattern `test/**/*.rb`
#   makes rake boot (test/dummy/config/environment.rb), which
#   current_attributes_test.rb wraps its cases in
#
# run.sh loads this file with `ruby -r` before each file, after helper.rb, as
# rake does. A library a file needs and is not listed here shows as an error
# on both servers.
require "helper"
require "sidekiq/api"
require "rack/session"
require "dummy/config/environment"
