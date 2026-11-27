char *literal = "ab";
const char *kept = "ab";
static const int c = 1;
int *cast = (int *)&c;
const int *read = &c;
char *table[2] = {"a", "b"};
const char *names[2] = {"a", "b"};

void unknown(void *p);

int main(void) {
    static char *inner = "x";
    unknown(literal);
    unknown(inner);
    return 0;
}
