void *malloc(int n);
void free(void *p);
int cond(void);

int main(void) {
    int *p = malloc(4);
    int i = 0;
    while (i < 3) {
        if (p != 0) {
            if (cond()) {
                free(p);
                p = 0;
            }
        }
        i = i + 1;
    }
    if (p != 0) {
        free(p);
    }
    return 0;
}
