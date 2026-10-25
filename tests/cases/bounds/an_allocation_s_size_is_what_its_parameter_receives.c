void *malloc(char n);
int main(void) { int *p = malloc(257); if (p == 0) { return 1; } *p = 5; return 0; }
