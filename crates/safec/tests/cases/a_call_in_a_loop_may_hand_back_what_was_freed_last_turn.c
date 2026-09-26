void *malloc(int n);
void free(void *p);
int *get(void);

int main(void) {
    int i = 0;
    while (i < 2) {
        int *t = get();
        if (t == 0) {
            return 0;
        }
        int r = *t;
        free(t);
        i = i + r;
    }
    return 0;
}
