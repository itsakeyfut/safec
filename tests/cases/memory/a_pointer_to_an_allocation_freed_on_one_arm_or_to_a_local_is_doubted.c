void *malloc(int n);
void free(void *p);
int cond(void);

int main(void) {
    int x = 7;
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *q;
    if (cond()) {
        q = p;
        free(p);
    } else {
        q = &x;
    }
    return *q;
}
