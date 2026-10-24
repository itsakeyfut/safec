void *malloc(int n);

int main(void) { int *q = malloc(8); int **pp = &q; *pp += 1; return 0; }
