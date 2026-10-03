void *malloc(int n);
void free(void *p);
int *make(void);
void forget(void);

int main(void) {
    int *q = make();
    if (q == 0) {
        return 0;
    }
    forget();
    return *q;
}
