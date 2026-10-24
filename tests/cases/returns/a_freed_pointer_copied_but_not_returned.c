void *malloc(int n);
void free(void *p);

int *keep(void) {
    int *p = malloc(4);
    int *other = malloc(4);
    free(p);
    int *q = p;
    return other;
}
