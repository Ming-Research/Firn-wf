# frozen_string_literal: true

# A Minitest reporter that appends one line to the consumers record for each
# case as it finishes, so that a process that is killed leaves the cases it
# had finished (research/investigations/consumers/):
#
#   FILE::CLASS#NAME<TAB>STATUS
#
# STATUS is pass, fail, error or skip. run.sh loads it with
# `ruby -r .../minitest_record.rb TEST_FILE` and names the record in
# CASE_RECORD and the file being run in CASE_FILE.
require "minitest"

module CaseRecord
  STATUS = {"." => "pass", "F" => "fail", "E" => "error", "S" => "skip"}.freeze

  class Reporter < Minitest::AbstractReporter
    def initialize(path, file)
      super()
      @path = path
      @file = file
    end

    def record(result)
      case_name = "#{@file}::#{result.klass}##{result.name}".gsub(/[\t\r\n]+/, " ")
      status = STATUS.fetch(result.result_code, "error")
      # One write of a whole line, so that a kill leaves no half line.
      File.open(@path, "a") { |record| record.write("#{case_name}\t#{status}\n") }
    end
  end
end

module Minitest
  def self.plugin_case_record_init(_options)
    reporter << CaseRecord::Reporter.new(ENV.fetch("CASE_RECORD"), ENV.fetch("CASE_FILE"))
  end

  extensions << "case_record"
end
