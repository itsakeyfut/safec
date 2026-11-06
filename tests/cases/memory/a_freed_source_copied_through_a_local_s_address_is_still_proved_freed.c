void *malloc(int n); void free(void *p);
void *memcpy(void *d, void *s, int n);
int main(void) {
    int *x = malloc(4);
    int **p = malloc(8);
    int **b = malloc(8);
    if (!x || !p || !b) return 1;
    *b = x;
    free(x);
    free(b);
    int ***t = &p;
    memcpy(*t, b, 8);
    free(p);
    return 0;
}
