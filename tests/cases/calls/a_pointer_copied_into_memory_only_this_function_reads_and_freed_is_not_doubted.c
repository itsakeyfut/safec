void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

int main(void) {
    int *x = malloc(4);
    int **b = malloc(8);
    int **box = malloc(8);
    if (!x || !b || !box) return 1;
    *b = x;
    memcpy(box, b, 8);
    free(b);
    free(x);
    free(box);
    return 0;
}
