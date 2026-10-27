void *malloc(int n);
void free(void *p);

int main(void) {
    int **b = malloc(8);
    if (b == 0) {
        return 1;
    }
    int *x = malloc(4);
    if (x == 0) {
        free(b);
        return 1;
    }
    free(x);
    *b = x;
    int *q = *b;
    free(b);
    return *q;
}
