void *malloc(int n);
void *memset(void *d, int c, char n);
int main(void) { char *p = malloc(4); if (p == 0) { return 1; } memset(p, 0, 260); return 0; }
