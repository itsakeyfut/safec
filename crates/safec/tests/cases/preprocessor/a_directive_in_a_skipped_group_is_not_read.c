#ifdef U
#pragma pack(4)
#include <stdio.h>
#if 1
#else
#elif 2
#endif
#define F(x) x
#error not compiled
#endif

int main(void) { return 0; }
