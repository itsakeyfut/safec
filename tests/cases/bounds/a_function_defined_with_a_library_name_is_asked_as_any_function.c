void *malloc(int n);
int memcmp(int *a, int *b, int n) { if (a != 0) { *a = 1; } return 0; }
int main(void) { char *c = malloc(1); if (c == 0) { return 1; } void *v = c; memcmp(v, v, 1); return 0; }
