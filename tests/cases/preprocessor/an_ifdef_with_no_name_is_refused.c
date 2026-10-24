#ifdef
int main(void) { return 1; }
#endif
#ifndef 1
int main(void) { return 2; }
#endif

int main(void) { return 0; }
