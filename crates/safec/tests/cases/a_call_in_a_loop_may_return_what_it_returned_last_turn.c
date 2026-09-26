void *malloc(int n);
void free(void *p);
void keep(int **where);
int *get(void);

int main(void) {
    int *slot = 0;
    keep(&slot);
    int i = 0;
    int sum = 0;
    while (i < 2) {
        int *t = get();
        if (t == 0) {
            return 0;
        }
        sum = sum + *t;
        int **x = &slot;
        int ***ppp = &x;
        **ppp = t;
        free(t);
        i = i + 1;
    }
    return sum;
}
