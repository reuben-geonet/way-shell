CC = gcc
CFLAGS += -g3 -Wall -MMD -MP
SOURCES = $(shell find -type f -regex ".*\.c")
OBJS = $(subst .c,.o,$(SOURCES))
OBJS += ../lib/cmd_tree/cmd_tree.o

way-sh: $(OBJS)
	$(CC) $(CFLAGS) $(LDFLAGS) -o way-sh $(OBJS)

clean:
	rm -rf way-sh
	rm -f $(OBJS) $(OBJS:.o=.d)

-include $(OBJS:.o=.d)
