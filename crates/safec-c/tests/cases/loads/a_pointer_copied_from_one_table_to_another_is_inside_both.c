void *malloc(int n);
void drop_inner(int **b);

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
    *a = p;
    *b = *a;
    drop_inner(b);
    return *p;
}
