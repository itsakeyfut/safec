static const char *p = "a";
static const int k = 1;
static const int *kp = &k;
static char c;
static int x;

char **one_level_down = (char **)&p;
void *through_void = &p;
struct Holder {
    char **pp;
} member = {(char **)&p};
char **table[2] = {0, (char **)&p};
int **an_integer_chain = (int **)&kp;
char ***a_compound_literal = &(char **){(char **)&p};

int *narrower = (int *)&c;

const char *const *every_level_const = &p;
static const char *names[2] = {"a", "b"};
int *const a_const_static = &x;
