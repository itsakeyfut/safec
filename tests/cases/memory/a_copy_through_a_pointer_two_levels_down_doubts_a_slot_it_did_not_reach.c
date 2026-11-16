void *malloc(int n); void *memcpy(void *d, void *s, int n); void *memmove(void *d, void *s, int n); char *strcpy(char *d, char *s); void free(void *p);
int f(int **x) {
    int ***box = malloc(16);
    int **b = malloc(16);
    if (!box || !b) return 0;
    if (!x) return 0;
    box[0] = b;
    box[1] = b;
    int **d = *box;
    if (!d) return 0;
    memcpy(*box, x, 8);
    int *p = b[1];
    free(b);
    free(box);
    return p == 0;
}
