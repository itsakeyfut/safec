void *malloc(int n);
void free(void *p);
int main(void) {
    int *x = malloc(4);
    int **p = malloc(8);
    if (!x || !p) return 1;
    int ***t = &p;
    int **q = *t; *q = x;
    free(x);
    free(p);
    return 0;
}
