COQC := coqc
COQFLAGS := -w -deprecated -R . Vibehouse

SOURCES := PtcQuorum.v BuilderPayments.v ForkChoice.v
VOS := $(SOURCES:.v=.vo)

.PHONY: all clean

all: $(VOS)

%.vo: %.v
	$(COQC) $(COQFLAGS) $<

clean:
	rm -f *.vo *.vok *.vos *.glob .*.aux

# Dependencies
BuilderPayments.vo: BuilderPayments.v
PtcQuorum.vo: PtcQuorum.v
ForkChoice.vo: ForkChoice.v
