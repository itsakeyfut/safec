void free(void *p);
void *malloc(int n);
int **pick(int **a, int **b);

int g(void) {
    int *x = 0;
    int *y = 0;
    int **r = pick(&x, &y);
    int *p = malloc(4);
    *r = p;
    free(p);
    return *y;
}
