void *malloc(int n);
void show(int *p);

int main(void) {
    int **a = malloc(8);
    if (a == 0) {
        return 0;
    }
    int **b = malloc(8);
    if (b == 0) {
        return 0;
    }
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    *a = p;
    *b = r;
    show(*b);
    return *p;
}
