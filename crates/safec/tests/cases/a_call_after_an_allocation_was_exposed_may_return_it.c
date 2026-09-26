void *malloc(int n);
void free(void *p);
void show(int *p);
int *make(void);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    show(p);
    int *q = make();
    if (q != 0) {
        *q = 1;
    }
    return 0;
}
