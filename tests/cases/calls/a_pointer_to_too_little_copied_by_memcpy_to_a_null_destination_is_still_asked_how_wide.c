void *malloc(int n); void free(void *p); void *memcpy(void *d, void *s, int n);
void put(void) {
    int *c;
    int **b = &c;
    c = malloc(1);
    memcpy(0, b, 8);
}
