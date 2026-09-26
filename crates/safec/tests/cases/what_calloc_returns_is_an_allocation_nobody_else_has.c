void *calloc(int n, int m);
void free(void *p);
int abs(int x);

int main(void) {
    int *q = calloc(1, 4);
    if (q == 0) {
        return 0;
    }
    int a = abs(-1);
    *q = a;
    free(q);
    return 0;
}
