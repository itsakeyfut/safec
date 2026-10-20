#define A
#ifdef A
#ifndef B
int chosen(void) { return 0; }
#else
int other(void) { return 1; }
#endif
#else
int other(void) { return 2; }
#endif
#ifndef A
int other(void) { return 3; }
#else
int also(void) { return 4; }
#endif

int main(void) { return chosen() + also(); }
