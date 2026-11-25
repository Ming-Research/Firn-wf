# frozen_string_literal: true

# The job the heartbeat probe of run.sh enqueues and a real `sidekiq` process
# runs, `sidekiq -r .../sidekiq_heartbeat_job.rb`: it does nothing, so that
# the process has processed a job when its first heartbeat flushes the
# metrics.
require "sidekiq"

class HeartbeatNoop
  include Sidekiq::Job

  def perform
  end
end
