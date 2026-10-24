void *malloc(int n);
char *strcpy(char *d, char *s);
int strlen(char *s);

int main(void) {
    char *s = malloc(8);
    char *t = malloc(8);
    int x = 0;
    if (s == 0) {
        return 0;
    }
    if (t == 0) {
        return 0;
    }
    t[0] = 0;
    if (strlen(strcpy(s, t)) > 0) {
        x = 1;
    }
    return x;
}
