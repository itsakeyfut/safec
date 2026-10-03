void *malloc(int n);
void free(void *p);
void drop_inner(int **box);
int pick(void);

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
    int **t = a;
    if (pick()) {
        t = b;
    }
    *t = p;
    drop_inner(b);
    return *p;
}
