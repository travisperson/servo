define DEF_SUBMODULE_TEST_RULES
# check target
check-$(1) : $$(DONE_$(1))
	@$$(call E, make check: $(1))

	$$(Q) \
	$$(ENV_CFLAGS_$(1)) \
	$$(ENV_RFLAGS_$(1)) \
	$$(MAKE) -C $$(B)src/$(1) check

DEPS_CHECK += check-$(1)
endef

$(foreach submodule,$(CFG_SUBMODULES),\
$(eval $(call DEF_SUBMODULE_TEST_RULES,$(submodule))))


# Testing targets

servo-test: $(DEPS_servo)
	$(RUSTC) $(RFLAGS_servo) --test -o $@ $<

reftest: $(S)src/reftest/reftest.rs servo
	$(RUSTC) $(RFLAGS_servo) -o $@ $< -L .

contenttest: $(S)src/contenttest/contenttest.rs servo
	$(RUSTC) $(RFLAGS_servo) -o $@ $< -L .

.PHONY: check $(DEPS_CHECK)

check: $(DEPS_CHECK) check-servo

check-servo: servo-test
	./servo-test $(TESTNAME)

check-ref: reftest
	./reftest --source-dir=$(S)/src/test/ref --work-dir=src/test/ref $(TESTNAME)

check-content: contenttest
	./contenttest --source-dir=$(S)/src/test/content $(TESTNAME)
