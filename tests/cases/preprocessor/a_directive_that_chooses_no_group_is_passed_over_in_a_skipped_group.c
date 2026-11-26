#if 0
#
# 40 "gen.c"
#define F(x) x
#undef F
#include <stdio.h>
#pragma pack(4)
#line 1
#error not compiled
#warning not compiled
#ident "x"
#sccs "x"
#assert machine(x86)
#unassert machine
#import "x.h"
#include_next "x.h"
#embed "x.bin"
#endif

int main(void) { return 0; }
