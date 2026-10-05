void *malloc(int n);
void free(void *p);
int use2(int **pp);
int cond(void);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    if (cond()) {
        free(a);
    }
    return use2(&a);
}
