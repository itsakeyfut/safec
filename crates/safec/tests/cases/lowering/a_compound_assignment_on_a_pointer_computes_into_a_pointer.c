void *malloc(int n);

int main(void) { int *p = malloc(8); p += 1; return 0; }
