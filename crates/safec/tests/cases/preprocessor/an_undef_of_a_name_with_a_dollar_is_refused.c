int A = 7;
#define A 1
#undef A$B

int g(void) { return A; }
